# PelagoDB Rust Client SDK

## Layout
- `pelagodb-client/`: Rust crate with tonic-generated gRPC client + convenience wrapper.

## Build
```bash
cd clients/rust/pelagodb-client
cargo build
```

## Use
```rust
use pelagodb_client::PelagoClient;
use pelagodb_client::{value_int, value_string};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = PelagoClient::connect("http://127.0.0.1:27615", "default", "demo").await?;
    let node = client
        .create_node("Person", [("name", value_string("Alice")), ("age", value_int(31))])
        .await?;
    println!("{}", node.id);
    Ok(())
}
```
