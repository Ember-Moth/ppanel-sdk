//! # ppanel-sdk — PPanel Plugin SDK for Rust
//!
//! Event-loop async runtime: goroutines as thread pool, `async fn` + `.await`.
//!
//! ```ignore
//! use ppanel_sdk::prelude::*;
//!
//! #[ppanel_sdk::handler]
//! async fn fetch(req: HandleRequest) -> HandleResponse {
//!     let data = host::http::get("https://api.example.com").await;
//!     HandleResponse { status: 200, body: data.unwrap().body, headers: Default::default() }
//! }
//! ```

extern crate alloc;

pub mod abi;
pub mod host;
pub mod prelude;
pub mod runtime;

include!(concat!(env!("OUT_DIR"), "/ppanel.plugin.v1.rs"));

pub use ppanel_sdk_macros::{event_handler, handler, init, middleware, start, stop};
pub use prost;
pub use prost_types;
