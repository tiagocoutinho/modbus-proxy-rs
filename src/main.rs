use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, Result};
use tokio::net::{TcpListener, TcpStream};

async fn handle_client(client: &mut TcpStream, modbus: &mut TcpStream) -> Result<()> {
    let (client_reader, mut client_writer) = client.split();
    let (modbus_reader, mut modbus_writer) = modbus.split();
    let mut client_reader = BufReader::new(client_reader);
    let mut modbus_reader = BufReader::new(modbus_reader);
    let mut header = vec![0; 4];
    let mut buffer = [0; 8192];
    loop {
        // Read header
        client_reader.read_exact(&mut header).await?;
        // Read size
        let size = client_reader.read_u16().await?;
        let u_size = size as usize;

        // Read payload
        client_reader.read_exact(&mut buffer[0..u_size]).await?;

        // Write all
        modbus_writer.write_all(&header).await?;
        modbus_writer.write_u16(size).await?;
        modbus_writer.write_all(&buffer[0..u_size]).await?;

        // Read header
        modbus_reader.read_exact(&mut header).await?;
        // Read size

        let size = modbus_reader.read_u16().await?;
        let u_size = size as usize;

        // Read payload
        //let mut buffer = vec![0; size as usize];
        modbus_reader.read_exact(&mut buffer[0..u_size]).await?;

        // Write all
        client_writer.write_all(&header).await?;
        client_writer.write_u16(size).await?;
        client_writer.write_all(&buffer[0..u_size]).await?;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    loop {
        let (mut client, _addr) = listener.accept().await?;
        let mut modbus = TcpStream::connect("127.0.0.1:5030").await?;
        client.set_nodelay(true)?;
        modbus.set_nodelay(true)?;

        tokio::spawn(async move {
            handle_client(&mut client, &mut modbus).await;
        });
    }
}
