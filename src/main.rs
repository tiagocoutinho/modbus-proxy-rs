use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Result};
use tokio::net::{TcpListener, TcpStream};

trait PacketSize {
    fn payload_size(&self) -> std::result::Result<u16, Box<dyn Error>>;

    fn total_size(&self) -> std::result::Result<u16, Box<dyn Error>> {
        Ok(6 + self.payload_size()?)
    }
}

impl PacketSize for [u8] {
    fn payload_size(&self) -> std::result::Result<u16, Box<dyn Error>> {
        Ok(u16::from_be_bytes(self[4..6].try_into()?))
    }
}

// async fn read_packet(stream: &TcpStream) -> Result<Packet> {}

async fn handle_client(
    client: &mut TcpStream,
    modbus: &mut TcpStream,
) -> std::result::Result<(), Box<dyn Error>> {
    let mut buffer = [0; 8192];
    loop {
        // Read header
        client.read_exact(&mut buffer[0..6]).await?;
        // calculate payload size
        let total_size = buffer.total_size()? as usize;

        client.read_exact(&mut buffer[6..total_size]).await?;

        // Write all
        modbus.write_all(&buffer[0..total_size]).await?;

        // Read header
        modbus.read_exact(&mut buffer[0..6]).await?;

        let total_size = buffer.total_size()? as usize;

        // Read payload
        modbus.read_exact(&mut buffer[6..total_size]).await?;

        // Write all
        client.write_all(&buffer[0..total_size]).await?;
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
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let modbus = Modbus {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    Ok(server(modbus).await?)
}
