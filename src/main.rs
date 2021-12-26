use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter, Result as IOResult};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};
use tokio::sync::{mpsc, oneshot};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;
type Frame = Vec<u8>;
type Sender = mpsc::Sender<(Frame, oneshot::Sender<Frame>)>;
type Receiver = mpsc::Receiver<(Frame, oneshot::Sender<Frame>)>;

struct Connection {
    writer: BufWriter<OwnedWriteHalf>,
    reader: BufReader<OwnedReadHalf>,
}

impl Connection {
    fn new(stream: TcpStream) -> Connection {
        let (reader, writer) = stream.into_split();
        Connection {
            writer: BufWriter::new(writer),
            reader: BufReader::new(reader),
        }
    }

    async fn read_frame_into(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Read header
        self.reader.read_exact(&mut buf[0..6]).await?;
        // calculate payload size
        let total_size = 6 + frame_size(&buf)?;
        self.reader.read_exact(&mut buf[6..total_size]).await?;
        Ok(total_size)
    }

    async fn write_frame(&mut self, buf: Frame) -> IOResult<()> {
        self.writer.write_all(&buf[..]).await?;
        self.writer.flush().await?;
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
        let frame = rx.await.unwrap();

        client.write_frame(frame).await?;
    }
}

struct Config {
    bind_address: String,
    modbus_address: String,
}

async fn run(modbus_address: String, channel: &mut Receiver) {
    let modbus = TcpStream::connect(modbus_address).await.unwrap();
    let mut modbus = Connection::new(modbus);
    let mut buf = [0; 8 * 1024];
    while let Some((frame, client)) = channel.recv().await {
        modbus.write_frame(frame).await;
        let size = modbus.read_frame_into(&mut buf).await.unwrap();
        client.send(buf[0..size].to_vec());
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
        let mut client = Connection::new(client);
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
