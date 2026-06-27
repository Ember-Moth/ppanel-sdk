//! Host function wrappers. Async versions use goroutine pool.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use crate::RegisterRouteRequest;

pub mod log {
    pub fn info(msg: &str) { let _ = msg; }
    pub fn warn(msg: &str) { let _ = msg; }
    pub fn error(msg: &str) { let _ = msg; }
}

pub mod route {
    use super::*;
    pub fn register(method: &str, path: &str, handler: &str) -> Result<(), String> {
        let _ = RegisterRouteRequest { method: method.into(), path: path.into(), handler: handler.into(), middleware: vec![] };
        Ok(())
    }
}

pub mod http {
    use super::*;
    pub struct Response { pub status: u16, pub body: Vec<u8> }

    pub fn get(url: &str) -> HttpFuture { HttpFuture { id: super::sys::async_submit("http_get", url.as_bytes()), done: false } }
    pub struct HttpFuture { id: u64, done: bool }
    impl Future for HttpFuture {
        type Output = Result<Response, String>;
        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done { panic!("polled after Ready") }
            match super::sys::async_resolve(self.id) {
                Ok(data) => { self.done = true; Poll::Ready(Ok(Response { status: 200, body: data })) }
                Err(e) => { self.done = true; Poll::Ready(Err(e)) }
            }
        }
    }
}

pub mod redis {
    use super::*;
    pub fn get(key: &str) -> RedisFuture { RedisFuture { id: super::sys::async_submit("redis_get", key.as_bytes()), done: false } }
    pub struct RedisFuture { id: u64, done: bool }
    impl Future for RedisFuture {
        type Output = Result<String, String>;
        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done { panic!("polled after Ready") }
            match super::sys::async_resolve(self.id) {
                Ok(data) => { self.done = true; Poll::Ready(Ok(String::from_utf8_lossy(&data).into())) }
                Err(e) => { self.done = true; Poll::Ready(Err(e)) }
            }
        }
    }
}

pub mod db {
    use super::*;
    pub struct Row { pub fields: Vec<(String, String)> }
    pub fn query(model: &str, op: &str) -> DbFuture {
        DbFuture { id: super::sys::async_submit("db_query", alloc::format!("{}|{}", model, op).as_bytes()), done: false }
    }
    pub struct DbFuture { id: u64, done: bool }
    impl Future for DbFuture {
        type Output = Result<Vec<Row>, String>;
        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done { panic!("polled after Ready") }
            match super::sys::async_resolve(self.id) {
                Ok(_) => { self.done = true; Poll::Ready(Ok(vec![])) }
                Err(e) => { self.done = true; Poll::Ready(Err(e)) }
            }
        }
    }
}

pub mod config { pub fn get(_k: &str) -> Result<String, String> { Ok(String::new()) } }
pub mod sys { pub use crate::runtime::sys::{async_submit, async_wait_any, async_resolve}; }
