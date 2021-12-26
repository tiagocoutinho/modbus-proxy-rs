use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter, Result as IOResult};
use tokio::net::{
    tcp::{OwnedReadHalf, OwnedWriteHalf},
    TcpListener, TcpStream,
};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

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
        let total_size = buf.total_size()?;
        self.reader.read_exact(&mut buf[6..total_size]).await?;
        Ok(total_size)
    }

    async fn write_frame(&mut self, buf: &[u8]) -> IOResult<()> {
        self.writer.write_all(buf).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

trait Packet {
    fn payload_size(&self) -> Result<usize>;

    fn total_size(&self) -> Result<usize> {
        Ok(6 + self.payload_size()?)
    }

    fn packet(&self) -> Result<&[u8]>;
}

impl Packet for [u8] {
    fn payload_size(&self) -> Result<usize> {
        Ok(u16::from_be_bytes(self[4..6].try_into()?) as usize)
    }
    fn packet(&self) -> Result<&[u8]> {
        let size = self.total_size()?;
        Ok(&self[0..size])
    }
}

async fn read_packet_into(stream: &mut TcpStream, buf: &mut [u8]) -> Result<usize> {
    // Read header
    stream.read_exact(&mut buf[0..6]).await?;
    // calculate payload size
    let total_size = buf.total_size()?;
    stream.read_exact(&mut buf[6..total_size]).await?;
    Ok(total_size)
}

async fn handle_client(client: &mut Connection, mut modbus: &mut TcpStream) -> Result<()> {
    let mut buf = [0; 8192];
    loop {
        let size = client.read_frame_into(&mut buf).await?;

        // Write all
        modbus.write_all(&buf[0..size]).await?;

        let size = read_packet_into(&mut modbus, &mut buf).await?;

        // Write all
        client.write_frame(&buf[0..size]).await?;
    }
}

struct Modbus {
    bind_address: String,
    modbus_address: String,
}

async fn server(modbus: Modbus) -> IOResult<()> {
    let listener = TcpListener::bind(modbus.bind_address).await?;

    loop {
        let addr = modbus.modbus_address.clone();
        let (client, client_addr) = listener.accept().await?;
        println!("Connecting to modbus for {}...", client_addr);
        let mut modbus = TcpStream::connect(addr).await?;
        println!("Connected to modbus for {}!", client_addr);
        // client.set_nodelay(true)?;
        // modbus.set_nodelay(true)?;
        let mut client = Connection::new(client);
        tokio::spawn(async move {
            match handle_client(&mut client, &mut modbus).await {
                Err(err) => eprintln!("Error {}: {:?}", client_addr, err),
                _ => {}
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> IOResult<()> {
    let modbus = Modbus {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    server(modbus).await
}
