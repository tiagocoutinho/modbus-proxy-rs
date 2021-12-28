use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

struct Config {
    bind_address: String,
    modbus_address: String,
}

type Frame = Vec<u8>;
type ReplySender = oneshot::Sender<Frame>;

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

fn frame_size(frame: &[u8]) -> Result<usize> {
    Ok(u16::from_be_bytes(frame[4..6].try_into()?) as usize)
}

fn split_connection(stream: TcpStream) -> (TcpReader, TcpWriter) {
    let (reader, writer) = stream.into_split();
    (BufReader::new(reader), BufWriter::new(writer))
}

async fn create_connection(address: &str) -> Result<(TcpReader, TcpWriter)> {
    Ok(split_connection(TcpStream::connect(address).await?))
}

async fn read_frame_into(stream: &mut TcpReader, buf: &mut [u8]) -> Result<usize> {
    // Read header
    stream.read_exact(&mut buf[0..6]).await?;
    // calculate payload size
    let total_size = 6 + frame_size(&buf)?;
    stream.read_exact(&mut buf[6..total_size]).await?;
    Ok(total_size)
}

async fn client_task(client: TcpStream, channel: ChannelTx) {
    channel.send(Message::Connection).await;
    let mut buf = [0; 8 * 1024];
    let (mut reader, mut writer) = split_connection(client);
    while let Ok(size) = read_frame_into(&mut reader, &mut buf).await {
        let (tx, rx) = oneshot::channel();
        let message = Message::Packet(buf[0..size].to_vec(), tx);
        if let Err(_) = channel.send(message).await {
            break;
        }
        match rx.await {
            Ok(frame) => {
                if let Err(err) = writer.write_all(&frame).await {
                    eprintln!("client: error writing reply to client (maybe disconnected?)");
                    break;
                }
                writer.flush().await.unwrap();
            }
            Err(err) => {
                eprintln!("client: error waiting for modbus reply: {}", err);
                break;
            }
        }
    }
    channel.send(Message::Disconnection).await;
}

async fn modbus_task(address: &str, channel: &mut ChannelRx) {
    let mut nb_clients = 0;
    let mut buf = [0; 8 * 1024];
    let mut modbus: Option<(TcpReader, TcpWriter)> = None;

    while let Some(message) = channel.recv().await {
        match message {
            Message::Connection => {
                println!("connecting to modbus at {}...", address);
                nb_clients += 1;
                let (reader, writer) = create_connection(address).await.unwrap();
                modbus = Some((reader, writer));
                println!("connected to modbus at {}!", address);
            }
            Message::Disconnection => {
                nb_clients -= 1;
                if nb_clients == 0 {
                    println!("disconnecting from modbus at {} (no clients)", address);
                    modbus = None;
                }
            }
            Message::Packet(frame, reply) => match modbus.as_mut() {
                Some((reader, writer)) => {
                    println!("sending to modbus {:?}", frame);
                    writer.write_all(&frame).await.unwrap();
                    writer.flush().await.unwrap();
                    let size = read_frame_into(reader, &mut buf).await.unwrap();
                    println!("{:?}", &buf[0..size]);
                    reply.send(buf[0..size].to_vec());
                }
                None => {}
            },
        }
    }
}

async fn bridge_task(config: Config) {
    let listener = TcpListener::bind(config.bind_address).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<Message>(32);

    tokio::spawn(async move {
        modbus_task(&config.modbus_address, &mut rx).await;
    });
    loop {
        let (mut client, client_addr) = listener.accept().await.unwrap();
        let tx = tx.clone();
        tokio::spawn(async move {
            client_task(client, tx).await;
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    //-> IOResult<()> {
    let config = Config {
        bind_address: "127.0.0.1:8080".to_string(),
        modbus_address: "127.0.0.1:5030".to_string(),
    };
    bridge_task(config).await
}
