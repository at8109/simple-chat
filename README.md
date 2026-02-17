# Simple Chat

A simple asynchronous chat server and CLI client built in Rust using Tokio.

## Architecture

The server uses the **Actor Model** pattern for high throughput:

- **Broker** — a single task managing all connected peers via channels (no locks)
- **Session** — one task per TCP connection, handling read/write concurrently
- **Client** — async CLI that splits stdin reading and server message display into separate tasks

The connection handler is generic over `AsyncRead + AsyncWrite`, making the core logic transport-agnostic.

## Usage

### Server

```bash
cargo run --bin server
```

The server listens on `127.0.0.1:8080` by default. Override with:

```bash
CHAT_ADDR=0.0.0.0:9000 cargo run --bin server
```

### Client

```bash
cargo run --bin client -- --username Alice
```

Options:

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--host` | `CHAT_HOST` | `127.0.0.1` | Server hostname |
| `--port` | `CHAT_PORT` | `8080` | Server port |
| `--username` | `CHAT_USERNAME` | *(required)* | Display name |

### Commands

| Command | Description |
|---------|-------------|
| `send <MSG>` | Send a message to all other users |
| `leave` | Disconnect and exit |

## Testing

```bash
cargo test          # unit + integration tests
cargo fmt --check   # formatting
cargo clippy        # lints
```

## Pre-commit Hook

Install [pre-commit](https://pre-commit.com/), then:

```bash
pre-commit install
```

This will run `cargo fmt`, `cargo check`, and `cargo clippy` before every commit.

## License

MIT
