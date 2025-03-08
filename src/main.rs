use clap::{Arg, Command};
use tokio::net::{UdpSocket, TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let matches = Command::new("drop")
        .version("1.0")
        .about("A peer-to-peer file sharing CLI")
        .subcommand(Command::new("listen").about("Listen for file offers from other peers"))
        .subcommand(
            Command::new("catch")
                .about("Catch a file shared by a peer")
                .arg(Arg::new("name").required(true).help("The name of the file to catch")),
        )
        .subcommand(
            Command::new("drop")
                .about("Broadcast a file for sharing")
                .arg(Arg::new("file").required(true).help("The file to share")),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("listen", _)) => listen().await,
        Some(("catch", sub_matches)) => {
            let name = sub_matches.get_one::<String>("name").unwrap();
            catch(name).await
        }
        Some(("drop", sub_matches)) => {
            let file = sub_matches.get_one::<String>("file").unwrap();
            drop_file(file.clone()).await
        }
        _ => Ok(()),
    }
}

async fn listen() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = UdpSocket::bind("0.0.0.0:9000").await?;
    println!("Listening on 0.0.0.0:9000 for peers...");

    let mut buf = [0; 1024];

    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        let msg = String::from_utf8_lossy(&buf[..len]);

        println!("📢 Received message from {}: {}", addr, msg);
    }
}

async fn drop_file(file: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;

    let broadcast_msg = format!("File available: {}", file);

    tokio::spawn(async move {
        loop {
            if let Err(e) = socket.send_to(broadcast_msg.as_bytes(), "255.255.255.255:9000").await {
                eprintln!("Failed to send broadcast: {}", e);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    println!("📡 Broadcasting file '{}' ", file);

    let listener = TcpListener::bind("0.0.0.0:9001").await?;
    println!("📥 Listening for file requests on 0.0.0.0:9001...");

    let file = Arc::new(file);
    loop {
        let (socket, _) = listener.accept().await?;
        let file_clone = Arc::clone(&file);
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, file_clone).await {
                eprintln!("Error handling client: {}", e);
            }
        });
    }
}


async fn handle_client(mut socket: TcpStream, file: Arc<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut file = File::open(&*file)?; // Arc<String> naar &str converteren
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    socket.write_all(&buffer).await?;
    println!("✅ File sent successfully!");

    Ok(())
}

async fn catch(name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("🔍 Looking for '{}' on the network...", name);

    let socket = UdpSocket::bind("0.0.0.0:9000").await?;
    let mut buf = [0; 1024];

    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        let msg = String::from_utf8_lossy(&buf[..len]);

        if msg.contains(name) {
            println!("🎯 Found file '{}' at peer: {}", name, addr);

            let peer_ip = addr.ip().to_string();
            download_file(&peer_ip).await?;
            break;
        }
    }

    Ok(())
}

async fn download_file(peer_ip: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:9001", peer_ip);
    let mut socket = TcpStream::connect(addr).await?;

    let mut buffer = Vec::new();
    socket.read_to_end(&mut buffer).await?;

    let mut file = File::create("received_file.txt")?;
    file.write_all(&buffer)?;

    println!("📥 File '{}' downloaded successfully!", "received_file.txt");
    Ok(())
}
