use clap::{Arg, Command};
use tokio::net::{UdpSocket, TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::fs::File;
use std::io::{self, Read, Write};
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::{DateTime, Utc, Local};
use std::time::Duration;
use std::path::Path;
use std::ffi::OsStr;
            
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
    let successful_downloads = Arc::new(Mutex::new(0)); // Track successful downloads
    let logs = Arc::new(Mutex::new(Vec::new())); // To store logs

    let socket_clone = Arc::clone(&socket); // Arc clone
    let file_clone = Arc::new(file); // Wrap file in Arc

    let last_broadcast_time = Arc::new(Mutex::new(None)); // Store last broadcast timestamp

    // Spawn a task to send broadcasts at regular intervals
    tokio::spawn({
        let file_clone = Arc::clone(&file_clone); // Clone for use in the closure
        let last_broadcast_time = Arc::clone(&last_broadcast_time);
        let counter_clone = Arc::clone(&counter); // Clone for use inside the closure
        let successful_downloads_clone = Arc::clone(&successful_downloads); // Clone for use inside the closure
        let logs_clone = Arc::clone(&logs); // Clone logs for use in the closure

        async move {
            let mut count = 1;
            loop {
                if let Err(e) = socket_clone.send_to(broadcast_msg.as_bytes(), "255.255.255.255:9000").await {
                    eprintln!("❌ Failed to send broadcast: {}", e);
                }

                // Update and display the broadcast timestamp
                let timestamp: DateTime<Utc> = Utc::now();
                let mut last_broadcast = last_broadcast_time.lock().await;
                *last_broadcast = Some(timestamp);

                // Clear the terminal and reprint everything
                print!("\x1b[2J\x1b[H"); // ANSI escape codes to clear the terminal and reset cursor position

                // Print the updated stats
                println!(
                    "📡 Last broadcast: {} | Active downloads: {} | Successful downloads: {}",
                    timestamp.format("%Y-%m-%d %H:%M:%S"),
                    *counter_clone.lock().await,
                    *successful_downloads_clone.lock().await
                );

                // Print the logs below the stats
                {
                    let log = logs_clone.lock().await;
                    for entry in log.iter() {
                        println!("{}", entry);
                    }
                }

                // Ensure the output is flushed immediately (so it appears)
                io::stdout().flush().unwrap();

                count += 1;
                tokio::time::sleep(Duration::from_secs(2)).await; // Wait 2 seconds before broadcasting again
            }
        }
    });

    println!("📡 Sharing '{}', waiting for download requests...", *file_clone);

    let listener = TcpListener::bind("0.0.0.0:9001").await?;

    loop {
        let (socket, _) = listener.accept().await?;
        let counter_clone = Arc::clone(&counter); // Clone before moving
        let successful_downloads_clone = Arc::clone(&successful_downloads); // Clone before moving
        let logs_clone = Arc::clone(&logs); // Clone logs for use in the task

        tokio::spawn({
            let file_clone = Arc::clone(&file_clone); // Clone for the task
            async move {
                {
                    let mut count = counter_clone.lock().await;
                    *count += 1; // Increment active downloads
                    // Log peer connection
                    println!("📥 Peer connected! Active downloads: {}", count);
                }

                let timestamp = Utc::now();

                // Log the download started event
                {
                    let mut log = logs_clone.lock().await;
                    log.push(format!("⏳ Download started at {}", timestamp));
                }

                if let Err(e) = handle_client(socket, file_clone.clone()).await {
                    eprintln!("❌ Error handling client: {}", e);
                    
                    // Log the download error event
                    {
                        let mut log = logs_clone.lock().await;
                        log.push(format!("❌ Download error at {}: {}", timestamp, e));
                    }
                }

                {
                    let mut count = counter_clone.lock().await;
                    *count -= 1; // Decrement active downloads after the file is sent
                    // Log file sent
                    println!("✅ File sent! Active downloads: {}", count);
                }

                {
                    let mut success = successful_downloads_clone.lock().await;
                    *success += 1; // Increment successful downloads
                    // Log total successful downloads
                    println!("✅ Total successful downloads: {}", success);
                }

                // Log the download success event
                {
                    let mut log = logs_clone.lock().await;
                    log.push(format!("✅ Download success at {}", Utc::now()));
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
        
            let filename = Path::new(name)
                .file_name()
                .unwrap_or_else(|| OsStr::new(name))  // Use OsStr::new for default value
                .to_str()
                .unwrap();  // Convert to &str


            match download_file(&peer_ip, filename).await {
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

async fn download_file(peer_ip: &str, filename: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:9001", peer_ip);
    println!("📡 Connecting to {} to download file '{}'...", addr, filename);

    let mut socket = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("❌ Failed to connect to peer: {}", e);
            return Err(e.into());
        }
    };

    let mut buffer: Vec<u8> = Vec::new();
    let mut file = File::create(filename)?;

    // Download the file in chunks (e.g., 64k chunks)
    let mut buf = vec![0; 64 * 1024]; // 64 KB buffer
    while let Ok(bytes_read) = socket.read(&mut buf).await {
        if bytes_read == 0 {
            break; // EOF
        }
        file.write_all(&buf[..bytes_read])?; // Write chunk to file
    }

    println!("✅ Successfully downloaded file! Saved as '{}'", filename);
    Ok(())
}
