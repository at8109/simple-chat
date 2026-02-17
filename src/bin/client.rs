use std::env;
use std::error::Error;

use clap::Parser;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LinesCodec};

/// CLI arguments for the chat client.
///
/// Values can also be provided via environment variables:
/// `CHAT_HOST`, `CHAT_PORT`, `CHAT_USERNAME`.
#[derive(Parser, Debug)]
#[command(author, version, about = "Simple async chat client")]
struct Args {
    /// Server hostname
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Server port
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// Your display name
    #[arg(long)]
    username: String,
}

/// Resolves CLI args with env‑var fallbacks for host, port, and username.
fn resolve_args() -> Args {
    let mut args = Args::parse();

    if let Ok(host) = env::var("CHAT_HOST") {
        args.host = host;
    }
    if let Ok(port) = env::var("CHAT_PORT")
        && let Ok(p) = port.parse::<u16>()
    {
        args.port = p;
    }
    if let Ok(username) = env::var("CHAT_USERNAME") {
        args.username = username;
    }
    args
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = resolve_args();
    let addr = format!("{}:{}", args.host, args.port);

    println!("Connecting to {} as '{}'...", addr, args.username);

    let socket = TcpStream::connect(&addr).await?;
    let mut lines = Framed::new(socket, LinesCodec::new_with_max_length(4096));

    // ── Handshake ──────────────────────────────────────────────────────────
    if let Some(Ok(msg)) = lines.next().await {
        println!("{msg}");
    }
    lines.send(&args.username).await?;

    // ── Split for concurrent read / write ──────────────────────────────────
    let (mut sink, mut stream) = lines.split();

    // Reader task: Server → stdout
    let reader_handle = tokio::spawn(async move {
        while let Some(result) = stream.next().await {
            match result {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("Error reading from server: {e}");
                    break;
                }
            }
        }
        println!("Disconnected from server.");
    });

    // Writer loop: stdin → Server
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        line.clear();
        if stdin.read_line(&mut line).await? == 0 {
            break; // EOF (e.g. piped input ended)
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match trimmed {
            "leave" => {
                sink.send("leave".to_string()).await?;
                break;
            }
            _ if trimmed.starts_with("send ") => {
                let msg = &trimmed["send ".len()..];
                sink.send(msg.to_string()).await?;
            }
            _ => {
                println!("Unknown command. Available commands:");
                println!("  send <MSG>  — send a message");
                println!("  leave       — disconnect and exit");
            }
        }
    }

    reader_handle.abort();
    Ok(())
}
