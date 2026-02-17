//! # Simple Chat Library
//!
//! Core logic for an asynchronous chat server using the Actor Model pattern.
//! The design follows SOLID principles:
//! - **SRP**: `Broker` manages state, `process_connection` manages I/O.
//! - **OCP**: Generic over `AsyncRead + AsyncWrite` — new transports need no changes here.
//! - **DIP**: Business logic depends on traits, not concrete types.

use std::collections::HashMap;

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LinesCodec};

/// Maximum allowed length for a single line (bytes). Prevents memory exhaustion.
const MAX_LINE_LENGTH: usize = 4096;

/// Maximum allowed length for a username (chars).
const MAX_USERNAME_LENGTH: usize = 32;

// ---------------------------------------------------------------------------
// Events – the messages flowing between Session tasks and the Broker actor.
// ---------------------------------------------------------------------------

/// Events that Session tasks send to the [`Broker`] actor.
#[derive(Debug)]
pub enum Event {
    /// A new user wants to join. Carries the desired username and a channel
    /// the Broker can use to send messages back to that user's Session.
    Join {
        username: String,
        sender: mpsc::UnboundedSender<String>,
    },
    /// A user has disconnected or typed `leave`.
    Leave { username: String },
    /// A user wants to broadcast a message to all other connected users.
    Message {
        sender_name: String,
        content: String,
    },
}

// ---------------------------------------------------------------------------
// Broker – the single source of truth for connected peers.
// ---------------------------------------------------------------------------

/// Runs the Broker actor loop.
///
/// The Broker owns the canonical map of connected peers and processes
/// [`Event`]s sequentially, eliminating the need for locks.
pub async fn broker_loop(mut rx: mpsc::UnboundedReceiver<Event>) {
    let mut peers: HashMap<String, mpsc::UnboundedSender<String>> = HashMap::new();

    while let Some(event) = rx.recv().await {
        match event {
            Event::Join { username, sender } => {
                handle_join(&mut peers, username, sender);
            }
            Event::Leave { username } => {
                handle_leave(&mut peers, &username);
            }
            Event::Message {
                sender_name,
                content,
            } => {
                broadcast(&peers, &sender_name, &content);
            }
        }
    }
}

/// Registers a new peer after ensuring the username is unique.
/// If the name is taken, a numeric suffix is appended (e.g. `Alice1`).
fn handle_join(
    peers: &mut HashMap<String, mpsc::UnboundedSender<String>>,
    username: String,
    sender: mpsc::UnboundedSender<String>,
) {
    let mut unique_name = username.clone();
    let mut counter = 1;
    while peers.contains_key(&unique_name) {
        unique_name = format!("{username}{counter}");
        counter += 1;
    }

    peers.insert(unique_name.clone(), sender.clone());
    tracing::info!("User joined: {}", unique_name);

    // Welcome the new user.
    let _ = sender.send(format!("Welcome! You are logged in as '{unique_name}'"));

    // Notify existing peers.
    for (name, tx) in peers.iter() {
        if *name != unique_name {
            let _ = tx.send(format!("{unique_name} has joined the chat"));
        }
    }
}

/// Removes a peer and notifies all remaining peers.
fn handle_leave(peers: &mut HashMap<String, mpsc::UnboundedSender<String>>, username: &str) {
    if peers.remove(username).is_some() {
        tracing::info!("User left: {}", username);
        for tx in peers.values() {
            let _ = tx.send(format!("{username} has left the chat"));
        }
    }
}

/// Sends a formatted message to every peer *except* the sender.
fn broadcast(
    peers: &HashMap<String, mpsc::UnboundedSender<String>>,
    sender_name: &str,
    content: &str,
) {
    for (name, tx) in peers.iter() {
        if *name != sender_name {
            let _ = tx.send(format!("{sender_name}: {content}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Server entry‑point
// ---------------------------------------------------------------------------

/// Binds a TCP listener on `addr` and spawns a Broker and per‑connection tasks.
pub async fn run_server(addr: &str) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on: {}", addr);

    let (broker_tx, broker_rx) = mpsc::unbounded_channel();
    tokio::spawn(broker_loop(broker_rx));

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        tracing::info!("Accepted connection from {}", peer_addr);
        let tx = broker_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(socket, tx).await {
                tracing::error!("Connection error ({}): {}", peer_addr, e);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Per‑connection handler – generic over any async byte stream.
// ---------------------------------------------------------------------------

/// Manages the lifecycle of a single client connection.
///
/// This function is generic over `S: AsyncRead + AsyncWrite`, which means it
/// can work with TCP, Unix sockets, or in‑memory streams (useful for testing).
pub async fn handle_connection<S>(stream: S, broker_tx: mpsc::UnboundedSender<Event>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut lines = Framed::new(stream, LinesCodec::new_with_max_length(MAX_LINE_LENGTH));

    // ── Handshake ──────────────────────────────────────────────────────────
    lines.send("Enter username:").await?;

    let username = match lines.next().await {
        Some(Ok(line)) => line.trim().to_string(),
        Some(Err(e)) => return Err(anyhow::anyhow!("Failed to read username: {e}")),
        None => {
            return Err(anyhow::anyhow!(
                "Client disconnected before sending username"
            ));
        }
    };

    if username.is_empty() || username.len() > MAX_USERNAME_LENGTH {
        lines
            .send(format!(
                "Invalid username. Must be 1‑{MAX_USERNAME_LENGTH} characters."
            ))
            .await?;
        return Ok(());
    }

    // ── Register with Broker ───────────────────────────────────────────────
    let (client_tx, mut client_rx) = mpsc::unbounded_channel();
    broker_tx.send(Event::Join {
        username: username.clone(),
        sender: client_tx,
    })?;

    // Split the framed stream so we can read and write concurrently.
    let (mut sink, mut stream) = lines.split();

    // ── Main event loop ────────────────────────────────────────────────────
    loop {
        tokio::select! {
            // Incoming data from the client socket.
            result = stream.next() => match result {
                Some(Ok(raw)) => {
                    let msg = raw.trim();
                    if msg == "leave" {
                        break;
                    }
                    if !msg.is_empty() {
                        broker_tx.send(Event::Message {
                            sender_name: username.clone(),
                            content: msg.to_string(),
                        })?;
                    }
                }
                Some(Err(e)) => {
                    tracing::error!("Read error from {}: {}", username, e);
                    break;
                }
                None => break, // Client closed the connection.
            },

            // Outgoing messages from the Broker destined for this client.
            msg = client_rx.recv() => match msg {
                Some(text) => {
                    if let Err(e) = sink.send(text).await {
                        tracing::error!("Write error to {}: {}", username, e);
                        break;
                    }
                }
                None => break, // Broker dropped — server shutting down.
            },
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    let _ = broker_tx.send(Event::Leave { username });
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Helper: creates a Broker channel pair, spawns the broker, and returns the sender.
    fn spawn_broker() -> mpsc::UnboundedSender<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(broker_loop(rx));
        tx
    }

    #[tokio::test]
    async fn test_broker_join_sends_welcome() {
        let broker_tx = spawn_broker();

        let (client_tx, mut client_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Alice".into(),
                sender: client_tx,
            })
            .unwrap();

        let msg = client_rx.recv().await.unwrap();
        assert!(msg.contains("Welcome"));
        assert!(msg.contains("Alice"));
    }

    #[tokio::test]
    async fn test_broker_broadcast_excludes_sender() {
        let broker_tx = spawn_broker();

        // Join Alice
        let (alice_tx, mut alice_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Alice".into(),
                sender: alice_tx,
            })
            .unwrap();
        let _ = alice_rx.recv().await; // consume Welcome

        // Join Bob
        let (bob_tx, mut bob_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Bob".into(),
                sender: bob_tx,
            })
            .unwrap();
        let _ = bob_rx.recv().await; // consume Welcome
        let _ = alice_rx.recv().await; // consume "Bob has joined"

        // Alice broadcasts
        broker_tx
            .send(Event::Message {
                sender_name: "Alice".into(),
                content: "Hello!".into(),
            })
            .unwrap();

        // Bob should receive it
        let msg = bob_rx.recv().await.unwrap();
        assert_eq!(msg, "Alice: Hello!");

        // Alice should NOT have a pending message (only the broadcast, which excludes her)
        // We use try_recv to avoid blocking
        assert!(alice_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_broker_leave_notifies_others() {
        let broker_tx = spawn_broker();

        // Join Alice
        let (alice_tx, mut alice_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Alice".into(),
                sender: alice_tx,
            })
            .unwrap();
        let _ = alice_rx.recv().await; // Welcome

        // Join Bob
        let (bob_tx, mut bob_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Bob".into(),
                sender: bob_tx,
            })
            .unwrap();
        let _ = bob_rx.recv().await; // Welcome
        let _ = alice_rx.recv().await; // "Bob has joined"

        // Bob leaves
        broker_tx
            .send(Event::Leave {
                username: "Bob".into(),
            })
            .unwrap();

        // Give broker a moment to process
        tokio::task::yield_now().await;

        let msg = alice_rx.recv().await.unwrap();
        assert!(msg.contains("Bob has left"));
    }

    #[tokio::test]
    async fn test_broker_duplicate_username_gets_suffix() {
        let broker_tx = spawn_broker();

        // Join Alice
        let (alice_tx, mut alice_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Alice".into(),
                sender: alice_tx,
            })
            .unwrap();
        let _ = alice_rx.recv().await; // Welcome

        // Join another Alice
        let (alice2_tx, mut alice2_rx) = mpsc::unbounded_channel();
        broker_tx
            .send(Event::Join {
                username: "Alice".into(),
                sender: alice2_tx,
            })
            .unwrap();

        let msg = alice2_rx.recv().await.unwrap();
        assert!(
            msg.contains("Alice1"),
            "Duplicate username should get suffix, got: {msg}"
        );
    }
}
