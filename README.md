# ppanel-sdk

PPanel plugin development SDK for Rust. Event-loop async runtime backed by Go goroutines.

## Quick Start

```rust
use ppanel_sdk::prelude::*;

#[ppanel_sdk::init]
fn init(req: InitRequest) -> Result<(), String> {
    host::route::register("GET", "/hello", "handle_hello")?;
    Ok(())
}

#[ppanel_sdk::handler]
async fn hello_handler(req: HandleRequest) -> HandleResponse {
    let data = host::http::get("https://api.example.com").await;
    HandleResponse { status: 200, body: data.unwrap().body, headers: Default::default() }
}
```

## Build

```bash
cargo build -p hello-world --target wasm32-unknown-unknown --release
```

## License

MIT
