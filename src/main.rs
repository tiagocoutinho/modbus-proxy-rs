use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Result};
use tokio::net::{TcpListener, TcpStream};

trait Packet {
    fn payload_size(&self) -> std::result::Result<usize, Box<dyn Error>>;

    fn total_size(&self) -> std::result::Result<usize, Box<dyn Error>> {
        Ok(6 + self.payload_size()?)
    }

    fn packet(&self) -> std::result::Result<&[u8], Box<dyn Error>>;
}

impl Packet for [u8] {
    fn payload_size(&self) -> std::result::Result<usize, Box<dyn Error>> {
        Ok(u16::from_be_bytes(self[4..6].try_into()?) as usize)
    }
    fn packet(&self) -> std::result::Result<&[u8], Box<dyn Error>> {
        let size = self.total_size()?;
        Ok(&self[0..size])
    }
}

async fn read_packet_into(
    stream: &mut TcpStream,
    buf: &mut [u8],
) -> std::result::Result<usize, Box<dyn Error>> {
    // Read header
    stream.read_exact(&mut buf[0..6]).await?;
    // calculate payload size
    let total_size = buf.total_size()?;

    stream.read_exact(&mut buf[6..total_size]).await?;
    Ok(total_size)
}

async fn write_packet(stream: &mut TcpStream, buf: &[u8], size: usize) -> Result<()> {
    stream.write_all(&buf[0..size]).await
}

async fn handle_client(
    mut client: &mut TcpStream,
    mut modbus: &mut TcpStream,
) -> std::result::Result<(), Box<dyn Error>> {
    let mut buf = [0; 8192];
    loop {
        let size = read_packet_into(&mut client, &mut buf).await?;

        // Write all
        write_packet(&mut modbus, &buf, size).await?;

        let size = read_packet_into(&mut modbus, &mut buf).await?;

        // Write all
        write_packet(&mut client, &buf, size).await?;
    }
}

struct Modbus {
    bind_address: String,
    modbus_address: String,
}

async fn server(modbus: Modbus) -> Result<()> {
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
async fn main() -> Result<()> {
    let modbus = Modbus {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    server(modbus).await
}
