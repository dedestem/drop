use clap::{Arg, Command};
use tokio::net::{UdpSocket, TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Local;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let matches = Command::new("drop")
        .version("1.0")
        .about("A peer-to-peer file sharing CLI")
        .subcommand(Command::new("listen").about("Listen for peers broadcasting files"))
        .subcommand(
            Command::new("catch")
                .about("Download a file shared by a peer")
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
    println!("👂 Listening on 0.0.0.0:9000 for file announcements...\n");

    let mut buf = [0; 1024];

    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        let msg = String::from_utf8_lossy(&buf[..len]);
        let timestamp = Local::now().format("%H:%M:%S");

        println!("[{}] 📢 {} from {}", timestamp, msg, addr);
    }
}

async fn drop_file(file: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    socket.set_broadcast(true)?;

    let broadcast_msg = format!("📢 File available: {}", file);
    let counter = Arc::new(Mutex::new(0));

    let socket_clone = Arc::clone(&socket); // Arc klonen
    let file_clone = Arc::new(file); // file in een Arc stoppen

    // Spawn een task om de broadcast te sturen
    tokio::spawn({
        let file_clone = Arc::clone(&file_clone); // Kloon voor gebruik binnen de closure
        async move {
            let mut count = 1;
            loop {
                if let Err(e) = socket_clone.send_to(broadcast_msg.as_bytes(), "255.255.255.255:9000").await {
                    eprintln!("❌ Failed to send broadcast: {}", e);
                }
                println!("📡 Broadcast #{}: '{}'", count, *file_clone); // Gebruik * om de waarde uit de Arc te halen
                count += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    });

    println!("📡 Sharing '{}', waiting for download requests...", *file_clone); // Gebruik * om de waarde uit de Arc te halen

    let listener = TcpListener::bind("0.0.0.0:9001").await?;

    loop {
        let (socket, _) = listener.accept().await?;

        // Clone de Arc voor elke nieuwe taak die je spawnt
        let counter_clone = Arc::clone(&counter);

        tokio::spawn({
            let file_clone = Arc::clone(&file_clone); // Maak een extra clone voor de closure
            async move {
                {
                    let mut count = counter_clone.lock().await;
                    *count += 1;
                    println!("📥 Peer connected! Active downloads: {}", count);
                }

                if let Err(e) = handle_client(socket, file_clone.clone()).await {
                    eprintln!("❌ Error handling client: {}", e);
                }

                {
                    let mut count = counter_clone.lock().await;
                    *count -= 1;
                    println!("✅ File sent! Remaining downloads: {}", count);
                }
            }
        });
    }
}

async fn handle_client(mut socket: TcpStream, file: Arc<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut file = File::open(&*file)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    socket.write_all(&buffer).await?;
    println!("✅ File sent successfully!");

    Ok(())
}

async fn catch(name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("🔍 Searching for '{}' on the network...", name);

    let socket = UdpSocket::bind("0.0.0.0:9000").await?;
    let mut buf = [0; 1024];

    let mut attempts = 0;
    loop {
        let (len, addr) = socket.recv_from(&mut buf).await?;
        let msg = String::from_utf8_lossy(&buf[..len]);

        if msg.contains(name) {
            println!("🎯 Found '{}' at peer: {}", name, addr);

            let peer_ip = addr.ip().to_string();
            match download_file(&peer_ip).await {
                Ok(_) => {
                    println!("✅ Successfully downloaded '{}'", name);
                    break;
                }
                Err(e) => {
                    eprintln!("❌ Failed to download file: {}. Retrying...", e);
                }
            }
        }

        attempts += 1;
        if attempts % 5 == 0 {
            println!("⏳ Still searching for '{}'... {} attempts made", name, attempts);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(())
}

async fn download_file(peer_ip: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:9001", peer_ip);
    println!("📡 Connecting to {} to download file...", addr);

    let mut socket = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("❌ Failed to connect to peer: {}", e);
            return Err(e.into());
        }
    };

    let mut buffer = Vec::new();
    socket.read_to_end(&mut buffer).await?;

    let filename = "received_file.txt";
    let mut file = File::create(filename)?;
    file.write_all(&buffer)?;

    println!("✅ Successfully downloaded file! Saved as '{}'", filename);
    Ok(())
}
