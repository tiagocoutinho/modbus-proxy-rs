use tokio::io::{AsyncReadExt, AsyncWriteExt, Result as IOResult};
use tokio::net::{TcpListener, TcpStream};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

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

async fn handle_client(mut client: &mut TcpStream, mut modbus: &mut TcpStream) -> Result<()> {
    let mut buf = [0; 8192];
    loop {
        let size = read_packet_into(&mut client, &mut buf).await?;

        // Write all
        modbus.write_all(&buf[0..size]).await?;

        let size = read_packet_into(&mut modbus, &mut buf).await?;

        // Write all
        client.write_all(&buf[0..size]).await?;
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
        let (mut client, _addr) = listener.accept().await?;
        let mut modbus = TcpStream::connect(addr).await?;
        // client.set_nodelay(true)?;
        // modbus.set_nodelay(true)?;

        tokio::spawn(async move {
            match handle_client(&mut client, &mut modbus).await {
                Err(err) => eprintln!("Error: {:?}", err),
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
