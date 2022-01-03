#[macro_use]
extern crate log;

use futures::future::join_all;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
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

fn frame_size(frame: &[u8]) -> Result<usize> {
    Ok(u16::from_be_bytes(frame[4..6].try_into()?) as usize)
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<Frame> {
    let mut buf = vec![0u8; 6];
    // Read header
    stream.read_exact(&mut buf).await?;
    // calculate payload size
    let total_size = 6 + frame_size(&buf)?;
    buf.resize(total_size, 0);
    stream.read_exact(&mut buf[6..total_size]).await?;
    Ok(buf)
}

#[derive(Debug, Deserialize)]
struct Listen {
    bind: String,
}

#[derive(Debug, Deserialize)]
struct Modbus {
    url: String,
}

struct Device {
    url: String,
    stream: Option<TcpStream>,
}

impl Device {
    pub fn new(url: &str) -> Device {
        Device {
            url: url.to_string(),
            stream: None,
        }
    }

    async fn connect(&mut self) -> Result<()> {
        match TcpStream::connect(&self.url).await {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                info!("modbus connection to {} sucessfull", self.url);
                self.stream = Some(stream);
                Ok(())
            }
            Err(error) => {
                self.stream = None;
                info!("modbus connection to {} error: {} ", self.url, error);
                Err(Box::new(error))
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
        let stream = self.stream.as_mut().ok_or("no modbus connection")?;
        stream.write_all(&frame).await?;
        stream.flush().await?;
        read_frame(stream).await
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

    async fn handle_packet(&mut self, frame: Frame, channel: ReplySender) -> Result<()> {
        info!("modbus request {}: {} bytes", self.url, frame.len());
        debug!("modbus request {}: {:?}", self.url, &frame[..]);
        let reply = self.write_read(&frame).await?;
        info!("modbus reply {}: {} bytes", self.url, reply.len());
        debug!("modbus reply {}: {:?}", self.url, &reply[..]);
        channel
            .send(reply)
            .or_else(|error| Err(format!("error sending reply to client: {:?}", error).into()))
    }

    async fn run(&mut self, channel: &mut ChannelRx) {
        let mut nb_clients = 0;

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
                        info!("disconnecting from modbus at {} (no clients)", self.url);
                        self.disconnect();
                    }
                }
                Message::Packet(frame, channel) => {
                    if let Err(_) = self.handle_packet(frame, channel).await {
                        self.disconnect();
                    }
                }
            }
        }
    }

    async fn launch(url: &str, channel: &mut ChannelRx) {
        let mut modbus = Self::new(url);
        modbus.run(channel).await;
    }
}

#[derive(Debug, Deserialize)]
struct Bridge {
    listen: Listen,
    modbus: Modbus,
}

impl Bridge {
    pub async fn run(&mut self) {
        let listener = TcpListener::bind(&self.listen.bind).await.unwrap();
        let modbus_url = self.modbus.url.clone();
        let (tx, mut rx) = mpsc::channel::<Message>(32);
        tokio::spawn(async move {
            Device::launch(&modbus_url, &mut rx).await;
        });
        info!(
            "Ready to accept requests on {} to {}",
            &self.listen.bind, &self.modbus.url
        );
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            client.set_nodelay(true).unwrap();
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(err) = Self::handle_client(&mut client, tx).await {
                    error!("Client error: {:?}", err);
                }
            });
        }
    }

    async fn handle_client(
        client: &mut (impl AsyncRead + AsyncWrite + Unpin),
        channel: ChannelTx,
    ) -> Result<()> {
        channel.send(Message::Connection).await?;
        while let Ok(buf) = read_frame(client).await {
            let (tx, rx) = oneshot::channel();
            channel.send(Message::Packet(buf, tx)).await?;
            client.write_all(&rx.await?).await?;
            client.flush().await?;
        }
        channel.send(Message::Disconnection).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct Server {
    devices: Vec<Bridge>,
}

impl Server {
    pub fn new(config_file: &str) -> std::result::Result<Self, config::ConfigError> {
        let mut cfg = config::Config::new();
        cfg.merge(config::File::with_name(config_file))?;
        cfg.try_into()
    }

    pub async fn run(self) {
        let mut tasks = vec![];
        for mut bridge in self.devices {
            tasks.push(tokio::spawn(async move { bridge.run().await }));
        }
        join_all(tasks).await;
    }

    pub async fn launch(config_file: &str) -> std::result::Result<(), config::ConfigError> {
        Ok(Self::new(config_file)?.run().await)
    }
}

#[test]
fn test_device_not_connected() {
    let device = Device::new("some url");
    assert_eq!(device.url, "some url");
    assert!(device.stream.is_none());
    assert_eq!(device.is_connected(), false);
}

#[test]
fn test_device_not_connected_disconnects() {
    let mut device = Device::new("some url");
    assert_eq!(device.is_connected(), false);
    device.disconnect();
    assert_eq!(device.is_connected(), false);
}

#[tokio::test]
async fn test_device_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
}
