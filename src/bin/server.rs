use anyhow::Result;
use smart_analytica_assignment::run_server;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let addr = std::env::var("CHAT_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    run_server(&addr).await
}
