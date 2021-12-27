use tokio::io::{AsyncReadExt, AsyncWriteExt, Result as IOResult};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;
type Frame = Vec<u8>;
type Reply = Option<Frame>;
type Sender = mpsc::Sender<(Frame, oneshot::Sender<Reply>)>;
type Receiver = mpsc::Receiver<(Frame, oneshot::Sender<Reply>)>;

struct Connection {
    address: String,
    stream: Option<TcpStream>,
}

impl Connection {
    fn new(address: &str) -> Connection {
        Connection {
            address: address.to_string(),
            stream: None,
        }
    }

    fn from_stream(stream: TcpStream, address: &str) -> Connection {
        Connection {
            address: address.to_string(),
            stream: Some(stream),
        }
    }

    async fn ensure_connection(&mut self) {
        if self.stream.is_none() {
            self.stream = TcpStream::connect(&self.address).await.ok();
        }
    }

    async fn read_frame_into(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.ensure_connection().await;
        if let Some(mut stream) = self.stream.take() {
            // Read header
            stream.read_exact(&mut buf[0..6]).await?;
            // calculate payload size
            let total_size = 6 + frame_size(&buf)?;
            stream.read_exact(&mut buf[6..total_size]).await?;
            self.stream = Some(stream);
            return Ok(total_size);
        }
        Err("no connection".into())
    }

    async fn write_frame(&mut self, buf: Frame) -> IOResult<()> {
        self.ensure_connection().await;
        if let Some(mut stream) = self.stream.take() {
            stream.write_all(&buf[..]).await?;
            self.stream = Some(stream);
        }
        Ok(())
    }
}

fn frame_size(frame: &[u8]) -> Result<usize> {
    Ok(u16::from_be_bytes(frame[4..6].try_into()?) as usize)
}

async fn handle_client(client: &mut Connection, channel: Sender) -> Result<()> {
    let mut buf = [0; 8 * 1024];
    loop {
        let size = client.read_frame_into(&mut buf).await?;
        let (tx, rx) = oneshot::channel();
        channel.send((buf[0..size].to_vec(), tx)).await.unwrap();
        if let Some(frame) = rx.await.unwrap() {
            client.write_frame(frame).await?;
        } else {
            return Err("error".into());
        }
    }
}

struct Config {
    bind_address: String,
    modbus_address: String,
}

async fn run(modbus_address: String, channel: &mut Receiver) {
    let mut modbus = Connection::new(&modbus_address);
    let mut buf = [0; 8 * 1024];
    while let Some((frame, client)) = channel.recv().await {
        let message = match modbus.write_frame(frame).await {
            Ok(_) => {
                let size = modbus.read_frame_into(&mut buf).await.unwrap();
                Some(buf[0..size].to_vec())
            }
            Err(err) => {
                eprintln!("Error writting frame to modbus device: {:?}", err);
                None
            }
        };
        if let Err(_) = client.send(message) {
            eprintln!("Error sending message to client (maybe went away?)");
        }
    }
}

async fn server(config: Config) -> IOResult<()> {
    let listener = TcpListener::bind(config.bind_address).await?;
    let (tx, mut rx) = mpsc::channel(32);

    let modbus_address = config.modbus_address;
    tokio::spawn(async move {
        run(modbus_address, &mut rx).await;
    });

    loop {
        let (client, client_addr) = listener.accept().await?;
        client.set_nodelay(true)?;
        let mut client = Connection::from_stream(client, &client_addr.to_string());
        let tx = tx.clone();
        tokio::spawn(async move {
            match handle_client(&mut client, tx).await {
                Err(err) => eprintln!("Error {}: {:?}", client_addr, err),
                _ => {}
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> IOResult<()> {
    let config = Config {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    server(config).await
}
