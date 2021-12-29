#[macro_use]
extern crate log;

use structopt::StructOpt;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, StructOpt)]
#[structopt(
    name = "modbus proxy",
    about = "Connect multiple clients to modbus devices"
)]
struct Config {
    #[structopt(short = "b", long = "bind", help = "listen TCP address (ex: 0:5020)")]
    bind_address: String,
    #[structopt(
        long = "modbus",
        help = "modbus device TCP address (ex: plc.acme.org:502)"
    )]
    modbus_address: String,
}

type Frame = Vec<u8>;
type ReplySender = oneshot::Sender<Frame>;

#[derive(Debug)]
enum Message {
    Connection,
    Disconnection,
    Packet(Frame, ReplySender),
}

type ChannelRx = mpsc::Receiver<Message>;
type ChannelTx = mpsc::Sender<Message>;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
type Result<T> = std::result::Result<T, Error>;

type TcpReader = BufReader<tokio::net::tcp::OwnedReadHalf>;
type TcpWriter = BufWriter<tokio::net::tcp::OwnedWriteHalf>;

struct Modbus {
    address: String,
    stream: Option<(TcpReader, TcpWriter)>,
}

impl Modbus {
    fn new(address: &str) -> Modbus {
        Modbus {
            address: address.to_string(),
            stream: None,
        }
    }

    async fn connect(&mut self) -> Result<()> {
        match create_connection(&self.address).await {
            Ok(connection) => {
                info!("modbus connection to {} sucessfull", self.address);
                self.stream = Some(connection);
                Ok(())
            }
            Err(error) => {
                self.stream = None;
                info!("modbus connection to {} error: {} ", self.address, error);
                Err(error)
            }
        }
    }

    fn disconnect(&mut self) {
        self.stream = None;
    }

    fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    async fn raw_write_read(&mut self, frame: &Frame) -> Result<Frame> {
        let (reader, writer) = self.stream.as_mut().ok_or("no modbus connection")?;
        writer.write_all(&frame).await?;
        writer.flush().await?;
        read_frame(reader).await
    }

    async fn write_read(&mut self, frame: &Frame) -> Result<Frame> {
        if self.is_connected() {
            let result = self.raw_write_read(&frame).await;
            match result {
                Ok(reply) => Ok(reply),
                Err(error) => {
                    warn!("modbus error: {}. Retrying...", error);
                    self.connect().await?;
                    self.raw_write_read(&frame).await
                }
            }
        } else {
            self.connect().await?;
            self.raw_write_read(&frame).await
        }
    }
}

fn frame_size(frame: &[u8]) -> Result<usize> {
    Ok(u16::from_be_bytes(frame[4..6].try_into()?) as usize)
}

fn split_connection(stream: TcpStream) -> (TcpReader, TcpWriter) {
    let (reader, writer) = stream.into_split();
    (BufReader::new(reader), BufWriter::new(writer))
}

async fn create_connection(address: &str) -> Result<(TcpReader, TcpWriter)> {
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    Ok(split_connection(stream))
}

async fn read_frame(stream: &mut TcpReader) -> Result<Frame> {
    let mut buf = vec![0u8; 6];
    // Read header
    stream.read_exact(&mut buf).await?;
    // calculate payload size
    let total_size = 6 + frame_size(&buf)?;
    buf.resize(total_size, 0);
    stream.read_exact(&mut buf[6..total_size]).await?;
    Ok(buf)
}

async fn client_task(client: TcpStream, channel: ChannelTx) -> Result<()> {
    client.set_nodelay(true)?;
    channel.send(Message::Connection).await?;
    let (mut reader, mut writer) = split_connection(client);
    while let Ok(buf) = read_frame(&mut reader).await {
        let (tx, rx) = oneshot::channel();
        channel.send(Message::Packet(buf, tx)).await?;
        writer.write_all(&rx.await?).await?;
        writer.flush().await?;
    }
    channel.send(Message::Disconnection).await?;
    Ok(())
}

async fn modbus_packet(modbus: &mut Modbus, frame: Frame, channel: ReplySender) -> Result<()> {
    info!("modbus request {}: {} bytes", modbus.address, frame.len());
    debug!("modbus request {}: {:?}", modbus.address, &frame[..]);
    let reply = modbus.write_read(&frame).await?;
    info!("modbus reply {}: {} bytes", modbus.address, reply.len());
    debug!("modbus reply {}: {:?}", modbus.address, &reply[..]);
    channel
        .send(reply)
        .or_else(|error| Err(format!("error sending reply to client: {:?}", error).into()))
}

async fn modbus_task(address: &str, channel: &mut ChannelRx) {
    let mut nb_clients = 0;
    let mut modbus = Modbus::new(address);

    while let Some(message) = channel.recv().await {
        match message {
            Message::Connection => {
                nb_clients += 1;
                info!("new client connection (active = {})", nb_clients);
            }
            Message::Disconnection => {
                nb_clients -= 1;
                info!("client disconnection (active = {})", nb_clients);
                if nb_clients == 0 {
                    info!("disconnecting from modbus at {} (no clients)", address);
                    modbus.disconnect();
                }
            }
            Message::Packet(frame, channel) => {
                if let Err(_) = modbus_packet(&mut modbus, frame, channel).await {
                    modbus.disconnect();
                }
            }
        }
    }
}

async fn bridge_task(config: Config) {
    let modbus_address = config.modbus_address.clone();
    let listener = TcpListener::bind(&config.bind_address).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<Message>(32);

    tokio::spawn(async move {
        modbus_task(&config.modbus_address, &mut rx).await;
    });
    info!(
        "Ready to accept requests on {} to {}",
        &config.bind_address, &modbus_address
    );
    loop {
        let (client, _) = listener.accept().await.unwrap();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(err) = client_task(client, tx).await {
                error!("Client error: {:?}", err);
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();
    let config = Config::from_args();
    bridge_task(config).await
}
