//! hello-world — PPanel example plugin

extern crate alloc;
use ppanel_sdk::prelude::*;

#[ppanel_sdk::init]
fn init(req: ppanel_sdk::InitRequest) -> Result<(), String> {
    let host = req.host_config.unwrap_or_default();
    host::log::info(&alloc::format!("hello-world started (host: {})", host.site_name));
    host::route::register("GET", "/hello", "handle_hello")?;
    host::route::register("GET", "/fetch", "handle_fetch")?;
    Ok(())
}

/// Sync handler — GET /v1/plugin/hello-world/hello
#[ppanel_sdk::handler]
fn hello_handler(req: ppanel_sdk::HandleRequest) -> ppanel_sdk::HandleResponse {
    let name = req.query.get("name").and_then(|v| v.values.first()).cloned().unwrap_or_else(|| "World".into());
    ppanel_sdk::HandleResponse { status: 200, body: alloc::format!(r#"{{"message":"Hello, {}!"}}"#, name).into_bytes(), headers: [("Content-Type".into(), "application/json".into())].into() }
}

/// Async handler — goroutines as thread pool
#[ppanel_sdk::handler]
async fn fetch_handler(req: ppanel_sdk::HandleRequest) -> ppanel_sdk::HandleResponse {
    let data = host::http::get("https://httpbin.org/json").await;
    let counter = host::redis::get("counter").await.unwrap_or_else(|_| "0".into());
    ppanel_sdk::HandleResponse { status: 200, body: alloc::format!(r#"{{"redis_counter":"{}","http_ok":{}}}"#, counter, data.is_ok()).into_bytes(), headers: [("Content-Type".into(), "application/json".into())].into() }
}
