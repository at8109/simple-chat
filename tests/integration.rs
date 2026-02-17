use futures::{SinkExt, StreamExt};
use smart_analytica_assignment::run_server;
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep, timeout};
use tokio_util::codec::{Framed, LinesCodec};

/// Helper: connect to the server and complete the handshake.
async fn connect(addr: &str, username: &str) -> Framed<TcpStream, LinesCodec> {
    let stream = TcpStream::connect(addr).await.expect("Failed to connect");
    let mut framed = Framed::new(stream, LinesCodec::new());

    // Server sends "Enter username:"
    let prompt = framed.next().await.unwrap().unwrap();
    assert_eq!(prompt, "Enter username:");

    // Send our username
    framed.send(username).await.unwrap();

    framed
}

/// Helper: read exactly one message from the framed stream, with a timeout.
async fn recv(client: &mut Framed<TcpStream, LinesCodec>) -> String {
    timeout(Duration::from_secs(5), client.next())
        .await
        .expect("Timed out waiting for message")
        .expect("Stream ended unexpectedly")
        .expect("Codec error")
}

#[tokio::test]
async fn test_broadcast_and_join_leave() {
    // ── Start server on a random available port ────────────────────────────
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener); // free the port so run_server can bind it

    tokio::spawn({
        let addr = addr.clone();
        async move {
            if let Err(e) = run_server(&addr).await {
                eprintln!("Server error: {e}");
            }
        }
    });
    sleep(Duration::from_millis(100)).await;

    // ── Connect Alice ──────────────────────────────────────────────────────
    let mut alice = connect(&addr, "Alice").await;
    let welcome = recv(&mut alice).await;
    assert!(
        welcome.contains("Welcome"),
        "Expected welcome, got: {welcome}"
    );

    // ── Connect Bob ────────────────────────────────────────────────────────
    let mut bob = connect(&addr, "Bob").await;
    let welcome = recv(&mut bob).await;
    assert!(
        welcome.contains("Welcome"),
        "Expected welcome, got: {welcome}"
    );

    // Alice should see "Bob has joined"
    let msg = recv(&mut alice).await;
    assert!(
        msg.contains("Bob has joined"),
        "Expected join notification, got: {msg}"
    );

    // ── Alice sends a message ──────────────────────────────────────────────
    alice.send("Hello Bob!").await.unwrap();

    // Bob receives it; Alice does NOT
    let msg = recv(&mut bob).await;
    assert_eq!(msg, "Alice: Hello Bob!");

    // ── Bob disconnects ────────────────────────────────────────────────────
    drop(bob);
    sleep(Duration::from_millis(100)).await;

    let msg = recv(&mut alice).await;
    assert!(
        msg.contains("Bob has left"),
        "Expected leave notification, got: {msg}"
    );
}

#[tokio::test]
async fn test_duplicate_username() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    tokio::spawn({
        let addr = addr.clone();
        async move {
            let _ = run_server(&addr).await;
        }
    });
    sleep(Duration::from_millis(100)).await;

    // Connect first Alice
    let mut alice1 = connect(&addr, "Alice").await;
    let _ = recv(&mut alice1).await; // Welcome

    // Connect second Alice — should get renamed to Alice1
    let mut alice2 = connect(&addr, "Alice").await;
    let welcome = recv(&mut alice2).await;
    assert!(
        welcome.contains("Alice1"),
        "Duplicate should be renamed, got: {welcome}"
    );
}

#[tokio::test]
async fn test_leave_command() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    tokio::spawn({
        let addr = addr.clone();
        async move {
            let _ = run_server(&addr).await;
        }
    });
    sleep(Duration::from_millis(100)).await;

    let mut alice = connect(&addr, "Alice").await;
    let _ = recv(&mut alice).await; // Welcome

    let mut bob = connect(&addr, "Bob").await;
    let _ = recv(&mut bob).await; // Welcome
    let _ = recv(&mut alice).await; // "Bob has joined"

    // Bob sends leave command
    bob.send("leave").await.unwrap();
    sleep(Duration::from_millis(100)).await;

    let msg = recv(&mut alice).await;
    assert!(
        msg.contains("Bob has left"),
        "Expected leave notification, got: {msg}"
    );
}
