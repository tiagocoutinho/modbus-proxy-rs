#[macro_use]
extern crate log;

use futures::future::join_all;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

// Use Jemalloc only for musl-64 bits platforms
#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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

#[derive(Debug, Deserialize)]
pub struct Listen {
    pub bind: String,
}

#[derive(Debug, Deserialize)]
pub struct Device {
    url: String,
    #[serde(skip)]
    stream: Option<(TcpReader, TcpWriter)>,
}

#[derive(Debug, Deserialize)]
pub struct Bridge {
    pub listen: Listen,
    pub modbus: Device,
}

#[derive(Debug, Deserialize)]
pub struct Server {
    pub bridges: Vec<Bridge>,
}

impl Server {
    pub fn new(config_file: &str) -> std::result::Result<Self, config::ConfigError> {
        let mut cfg = config::Config::new();
        cfg.merge(config::File::with_name(config_file)).unwrap();
        cfg.try_into()
    }

    pub async fn run(&mut self) {
        let tasks = self
            .bridges
            .drain(..)
            .map(|bridge| tokio::spawn(bridge.run()))
            .collect::<Vec<_>>();
        join_all(tasks).await;
    }

    pub async fn launch(config_file: &str) -> std::result::Result<(), config::ConfigError> {
        Ok(Self::new(config_file)?.run().await)
    }
}

impl Device {
    pub fn new(url: &str) -> Device {
        Device {
            url: url.to_string(),
            stream: None,
        }
    }

    async fn connect(&mut self) -> Result<()> {
        match create_connection(&self.url).await {
            Ok(connection) => {
                info!("modbus connection to {} sucessfull", self.url);
                self.stream = Some(connection);
                Ok(())
            }
            Err(error) => {
                self.stream = None;
                info!("modbus connection to {} error: {} ", self.url, error);
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

async fn create_connection(url: &str) -> Result<(TcpReader, TcpWriter)> {
    let stream = TcpStream::connect(url).await?;
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

async fn modbus_packet(device: &mut Device, frame: Frame, channel: ReplySender) -> Result<()> {
    info!("modbus request {}: {} bytes", device.url, frame.len());
    debug!("modbus request {}: {:?}", device.url, &frame[..]);
    let reply = device.write_read(&frame).await?;
    info!("modbus reply {}: {} bytes", device.url, reply.len());
    debug!("modbus reply {}: {:?}", device.url, &reply[..]);
    channel
        .send(reply)
        .or_else(|error| Err(format!("error sending reply to client: {:?}", error).into()))
}

async fn modbus_task(url: &str, channel: &mut ChannelRx) {
    let mut nb_clients = 0;
    let mut device = Device::new(url);

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
                    info!("disconnecting from modbus at {} (no clients)", url);
                    device.disconnect();
                }
            }
            Message::Packet(frame, channel) => {
                if let Err(_) = modbus_packet(&mut device, frame, channel).await {
                    device.disconnect();
                }
            }
        }
    }
}

impl Bridge {
    pub async fn run(self) {
        let modbus_url = self.modbus.url.clone();
        let listener = TcpListener::bind(&self.listen.bind).await.unwrap();
        let (tx, mut rx) = mpsc::channel::<Message>(32);

        tokio::spawn(async move {
            modbus_task(&modbus_url, &mut rx).await;
        });
        info!(
            "Ready to accept requests on {} to {}",
            &self.listen.bind, &self.modbus.url
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
}
