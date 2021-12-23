use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    loop {
        let (mut client, _) = listener.accept().await?;
        let mut modbus = TcpStream::connect("127.0.0.1:5030").await?;

        tokio::spawn(async move {
            let mut buf = [0; 1024];

            // In a loop, read data from the client and write the data back.
            loop {
                let n = match client.read(&mut buf).await {
                    // socket closed
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from client; err = {:?}", e);
                        return;
                    }
                };

                if let Err(e) = modbus.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to modbus; err = {:?}", e);
                    return;
                }

                let n = match modbus.read(&mut buf).await {
                    // socket closed
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from modbus; err = {:?}", e);
                        return;
                    }
                };

                // Write the data back
                if let Err(e) = client.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to client; err = {:?}", e);
                    return;
                }
            }
        });
    }
}
