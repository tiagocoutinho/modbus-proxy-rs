use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

struct Config {
    bind_address: String,
    modbus_address: String,
}

type Frame = Vec<u8>;
type ReplySender = oneshot::Sender<Frame>;

enum Message {
    Connection,
    Disconnection,
    Packet((Frame, ReplySender)),
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

    async fn reconnect(&mut self) {
        self.stream = create_connection(&self.address).await.ok();
    }

    fn disconnect(&mut self) {
        self.stream = None;
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
    Ok(split_connection(TcpStream::connect(address).await?))
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

async fn client_task(client: TcpStream, channel: ChannelTx) {
    channel.send(Message::Connection).await;
    let (mut reader, mut writer) = split_connection(client);
    while let Ok(buf) = read_frame(&mut reader).await {
        let (tx, rx) = oneshot::channel();
        let message = Message::Packet((buf, tx));
        if let Err(_) = channel.send(message).await {
            break;
        }
        match rx.await {
            Ok(frame) => {
                if let Err(err) = writer.write_all(&frame).await {
                    eprintln!(
                        "client: error writing reply to client (maybe disconnected?): {:?}",
                        err
                    );
                    break;
                }
                writer.flush().await.unwrap();
            }
            Err(err) => {
                eprintln!("client: error waiting for modbus reply: {}", err);
                break;
            }
        }
    }
    channel.send(Message::Disconnection).await;
}

async fn modbus_packet(modbus: &mut Modbus, packet: (Frame, ReplySender)) {
    if let Some((reader, writer)) = modbus.stream.as_mut() {
        println!("sending to modbus {:?}", packet.0);
        writer.write_all(&packet.0).await.unwrap();
        writer.flush().await.unwrap();
        let buf = read_frame(reader).await.unwrap();
        println!("{:?}", buf);
        packet.1.send(buf);
    }
}

async fn modbus_task(address: &str, channel: &mut ChannelRx) {
    let mut nb_clients = 0;
    let mut modbus = Modbus::new(address);

    while let Some(message) = channel.recv().await {
        match message {
            Message::Connection => {
                println!("connecting to modbus at {}...", address);
                nb_clients += 1;
                modbus.reconnect().await;
                println!("connected to modbus at {}!", address);
            }
            Message::Disconnection => {
                nb_clients -= 1;
                if nb_clients == 0 {
                    println!("disconnecting from modbus at {} (no clients)", address);
                    modbus.disconnect();
                }
            }
            Message::Packet(packet) => {
                modbus_packet(&mut modbus, packet).await;
            }
        }
    }
}

async fn bridge_task(config: Config) {
    let listener = TcpListener::bind(config.bind_address).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<Message>(32);

    tokio::spawn(async move {
        modbus_task(&config.modbus_address, &mut rx).await;
    });
    loop {
        let (client, _) = listener.accept().await.unwrap();
        let tx = tx.clone();
        tokio::spawn(async move {
            client_task(client, tx).await;
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let config = Config {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    bridge_task(config).await
}
