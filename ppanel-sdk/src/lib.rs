//! # ppanel-sdk — PPanel Plugin SDK for Rust
//!
//! Event-loop async runtime: goroutines as thread pool, `async fn` + `.await`.

extern crate alloc;

pub mod abi;
pub mod host;
pub mod prelude;
pub mod runtime;

include!(concat!(env!("OUT_DIR"), "/ppanel.plugin.v1.rs"));

pub use prost;
pub use ppanel_sdk_macros::{init, handler};
