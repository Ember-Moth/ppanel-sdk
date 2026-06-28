//! Host function wrappers.
//!
//! * **Synchronous** wrappers (`log`, `route`, `redis`, `config`, `events`,
//!   `http_sync`, `db_sync`, `scheduler`) call the host via real WASM imports
//!   and block the WASM thread until the host returns.
//! * **Asynchronous** wrappers (`http`, `redis_async`, `db_async`) submit work
//!   to the host's goroutine pool and return a `Future`.  Polling resolves the
//!   result.  With the **current** host (blocking `async_resolve`) the future
//!   is always `Ready` after the first poll; when the host moves to a
//!   non-blocking `async_resolve` the future will return `Pending` and the
//!   event loop in `runtime::block_on` will yield via `async_wait_any`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use prost::Message;

use crate::abi;

// ============================================================================
// Core ABI helper — synchronous host call
// ============================================================================

/// Call any synchronous host function: encode request → write WASM memory →
/// call (i32,i32)→i64 import → read response → decode.
fn call_host<T: Message, R: Message + Default>(
    f: unsafe extern "C" fn(i32, i32) -> i64,
    req: &T,
) -> R {
    let req_data = req.encode_to_vec();
    let packed = abi::encode_bytes(&req_data);
    if packed == 0 && req_data.len() > 0 {
        return R::default(); // allocation failure
    }
    let ptr = (packed >> 32) as i32;
    let len = (packed & 0xFFFF_FFFF) as i32;
    let result_packed = unsafe { f(ptr, len) };
    let result_ptr = (result_packed >> 32) as u32;
    let result_len = (result_packed & 0xFFFF_FFFF) as u32;
    if result_len == 0 {
        abi::deallocate(ptr, len);
        return R::default();
    }
    let resp_bytes =
        unsafe { core::slice::from_raw_parts(result_ptr as *const u8, result_len as usize) };
    let ret = R::decode(resp_bytes).unwrap_or_default();
    abi::deallocate(ptr, len);
    abi::deallocate(result_ptr as i32, result_len as i32);
    ret
}

// ============================================================================
// extern "C" declarations for all 9 synchronous host imports
// ============================================================================

mod imports {
    #[link(wasm_import_module = "env")]
    extern "C" {
        pub fn host_log(ptr: i32, len: i32) -> i64;
        pub fn host_config_get(ptr: i32, len: i32) -> i64;
        pub fn host_register_route(ptr: i32, len: i32) -> i64;
        pub fn host_register_middleware(ptr: i32, len: i32) -> i64;
        pub fn host_redis_get(ptr: i32, len: i32) -> i64;
        pub fn host_redis_set(ptr: i32, len: i32) -> i64;
        pub fn host_emit_event(ptr: i32, len: i32) -> i64;
        pub fn host_subscribe_event(ptr: i32, len: i32) -> i64;
        pub fn host_http_request(ptr: i32, len: i32) -> i64;
        pub fn host_schedule_task(ptr: i32, len: i32) -> i64;
        pub fn host_enqueue_task(ptr: i32, len: i32) -> i64;
        pub fn host_db_query(ptr: i32, len: i32) -> i64;
    }
}

// ============================================================================
// Public host function modules — synchronous
// ============================================================================

/// Logging — calls `host_log`.
pub mod log {
    use crate::{BoolResult, LogRequest};
    use alloc::collections::BTreeMap;

    pub fn info(msg: &str) {
        write("info", msg);
    }
    pub fn warn(msg: &str) {
        write("warn", msg);
    }
    pub fn error(msg: &str) {
        write("error", msg);
    }

    fn write(level: &str, msg: &str) {
        let req = LogRequest {
            level: level.into(),
            message: msg.into(),
            fields: BTreeMap::new(),
        };
        super::call_host::<LogRequest, BoolResult>(super::imports::host_log, &req);
    }
}

/// Route registration — calls `host_register_route`.
pub mod route {
    use crate::{BoolResult, RegisterRouteRequest};

    pub fn register(method: &str, path: &str, handler: &str) -> Result<(), alloc::string::String> {
        internal(method, path, handler, &[])
    }

    pub fn register_with_middleware(
        method: &str,
        path: &str,
        middleware: &str,
        handler: &str,
    ) -> Result<(), alloc::string::String> {
        internal(method, path, handler, &[middleware.into()])
    }

    fn internal(
        method: &str,
        path: &str,
        handler: &str,
        middleware: &[alloc::string::String],
    ) -> Result<(), alloc::string::String> {
        let req = RegisterRouteRequest {
            method: method.into(),
            path: path.into(),
            handler: handler.into(),
            middleware: middleware.to_vec(),
        };
        let resp: BoolResult = super::call_host(super::imports::host_register_route, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

/// Redis — calls `host_redis_get` / `host_redis_set`.
pub mod redis {
    use crate::{BoolResult, RedisGetRequest, RedisGetResponse, RedisSetRequest};

    /// Synchronous get.
    pub fn get(key: &str) -> Result<alloc::string::String, alloc::string::String> {
        let req = RedisGetRequest { key: key.into() };
        let resp: RedisGetResponse = super::call_host(super::imports::host_redis_get, &req);
        if resp.exists {
            Ok(resp.value)
        } else {
            Err("not found".into())
        }
    }

    /// Synchronous set.
    pub fn set(key: &str, value: &str, ttl_seconds: i64) -> Result<(), alloc::string::String> {
        let req = RedisSetRequest {
            key: key.into(),
            value: value.into(),
            ttl_seconds,
        };
        let resp: BoolResult = super::call_host(super::imports::host_redis_set, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

/// Config — calls `host_config_get`.
pub mod config {
    use crate::{ConfigGetRequest, ConfigGetResponse};

    pub fn get(key: &str) -> Result<alloc::string::String, alloc::string::String> {
        let req = ConfigGetRequest { key: key.into() };
        let resp: ConfigGetResponse = super::call_host(super::imports::host_config_get, &req);
        if resp.exists {
            Ok(resp.value)
        } else {
            Err("not found".into())
        }
    }
}

/// Events — calls `host_emit_event`.
pub mod events {
    use crate::{BoolResult, EmitEventRequest, SubscribeEventRequest};
    use prost_types::Struct;

    pub fn emit(event: &str, payload: Struct) -> Result<(), alloc::string::String> {
        let req = EmitEventRequest {
            event: event.into(),
            payload: Some(payload),
        };
        let resp: BoolResult = super::call_host(super::imports::host_emit_event, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }

    pub fn subscribe(event: &str, handler: &str) -> Result<(), alloc::string::String> {
        let req = SubscribeEventRequest {
            event: event.into(),
            handler: handler.into(),
        };
        let resp: BoolResult = super::call_host(super::imports::host_subscribe_event, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

/// HTTP client (synchronous) — calls `host_http_request`.
pub mod http_sync {
    use crate::{HttpRequestRequest, HttpRequestResponse};
    use alloc::collections::BTreeMap;
    use alloc::string::String;

    pub fn request(
        method: &str,
        url: &str,
        headers: BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<HttpRequestResponse, alloc::string::String> {
        let req = HttpRequestRequest {
            method: method.into(),
            url: url.into(),
            headers,
            body: body.to_vec(),
        };
        Ok(super::call_host(super::imports::host_http_request, &req))
    }
}

/// DB query (synchronous) — calls `host_db_query`.
pub mod db_sync {
    use crate::DbQueryRequest;
    use crate::DbQueryResponse;

    pub fn query(model: &str, operation: &str) -> Result<DbQueryResponse, alloc::string::String> {
        let req = DbQueryRequest {
            model: model.into(),
            operation: operation.into(),
            conditions: None,
            fields: None,
            limit: 0,
            offset: 0,
        };
        Ok(super::call_host(super::imports::host_db_query, &req))
    }

    pub fn query_raw(req: &DbQueryRequest) -> Result<DbQueryResponse, alloc::string::String> {
        Ok(super::call_host(super::imports::host_db_query, req))
    }
}

/// Scheduler — calls `host_schedule_task`.
pub mod scheduler {
    use crate::{BoolResult, ScheduleTaskRequest};

    pub fn register(name: &str, cron: &str, handler: &str) -> Result<(), alloc::string::String> {
        let req = ScheduleTaskRequest {
            name: name.into(),
            cron: cron.into(),
            handler: handler.into(),
        };
        let resp: BoolResult = super::call_host(super::imports::host_schedule_task, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

/// Middleware registration — calls `host_register_middleware`.
pub mod middleware {
    use crate::{BoolResult, RegisterMiddlewareRequest};

    pub fn register(name: &str, handler: &str) -> Result<(), alloc::string::String> {
        let req = RegisterMiddlewareRequest {
            name: name.into(),
            handler: handler.into(),
        };
        let resp: BoolResult = super::call_host(super::imports::host_register_middleware, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

/// Queue — calls `host_enqueue_task`.
pub mod queue {
    use crate::{BoolResult, EnqueueTaskRequest};
    use prost_types::Struct;

    pub fn enqueue(task_name: &str, payload: Struct) -> Result<(), alloc::string::String> {
        let req = EnqueueTaskRequest {
            task_name: task_name.into(),
            payload: Some(payload),
        };
        let resp: BoolResult = super::call_host(super::imports::host_enqueue_task, &req);
        if resp.success {
            Ok(())
        } else {
            Err(resp.error)
        }
    }
}

// ============================================================================
// Asynchronous wrappers — goroutine pool via runtime::sys
// ============================================================================
//
// NOTE: the current Go host blocks inside `host_async_resolve` until the
// goroutine finishes, so these futures always return `Ready` on the first
// poll.  The `Pending` arm exists so that when the host becomes non-blocking
// the futures will work correctly without any SDK changes.
// ============================================================================

/// Async HTTP client.
pub mod http {
    use super::*;
    use crate::{HttpRequestRequest, HttpRequestResponse};

    /// HTTP response returned by async `get` / `post`.
    pub struct Response {
        pub status: u16,
        pub body: Vec<u8>,
    }

    /// Submit an HTTP GET to the goroutine pool.  Returns a future.
    pub fn get(url: &str) -> HttpFuture {
        let req = HttpRequestRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Default::default(),
            body: vec![],
        };
        HttpFuture {
            id: super::sys::async_submit("http_get", &req.encode_to_vec()),
            done: false,
        }
    }

    /// Submit an HTTP POST to the goroutine pool.  Returns a future.
    pub fn post(url: &str, body: &[u8]) -> HttpFuture {
        let req = HttpRequestRequest {
            method: "POST".into(),
            url: url.into(),
            headers: Default::default(),
            body: body.to_vec(),
        };
        HttpFuture {
            id: super::sys::async_submit("http_post", &req.encode_to_vec()),
            done: false,
        }
    }

    pub struct HttpFuture {
        id: u64,
        done: bool,
    }

    impl Future for HttpFuture {
        type Output = Result<Response, String>;

        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done {
                panic!("polled after Ready")
            }
            match super::sys::async_resolve(self.id) {
                Ok(data) => {
                    self.done = true;
                    let resp = HttpRequestResponse::decode(data.as_slice()).unwrap_or_default();
                    Poll::Ready(Ok(Response {
                        status: resp.status as u16,
                        body: resp.body,
                    }))
                }
                Err(e) => {
                    if e == "pending" {
                        Poll::Pending
                    } else {
                        self.done = true;
                        Poll::Ready(Err(e))
                    }
                }
            }
        }
    }
}

/// Async Redis client.
pub mod redis_async {
    use super::*;
    use crate::{BoolResult, RedisGetRequest, RedisGetResponse, RedisSetRequest};

    /// Submit a Redis GET to the goroutine pool.
    pub fn get(key: &str) -> RedisFuture {
        let req = RedisGetRequest { key: key.into() };
        RedisFuture {
            id: super::sys::async_submit("redis_get", &req.encode_to_vec()),
            done: false,
        }
    }

    /// Submit a Redis SET to the goroutine pool.
    pub fn set(key: &str, value: &str, ttl_seconds: i64) -> RedisSetFuture {
        let req = RedisSetRequest {
            key: key.into(),
            value: value.into(),
            ttl_seconds,
        };
        RedisSetFuture {
            id: super::sys::async_submit("redis_set", &req.encode_to_vec()),
            done: false,
        }
    }

    // -- RedisGet future --

    pub struct RedisFuture {
        id: u64,
        done: bool,
    }

    impl Future for RedisFuture {
        type Output = Result<String, String>;

        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done {
                panic!("polled after Ready")
            }
            match super::sys::async_resolve(self.id) {
                Ok(data) => {
                    self.done = true;
                    let resp = RedisGetResponse::decode(data.as_slice()).unwrap_or_default();
                    if resp.exists {
                        Poll::Ready(Ok(resp.value))
                    } else {
                        Poll::Ready(Err("not found".into()))
                    }
                }
                Err(e) => {
                    if e == "pending" {
                        Poll::Pending
                    } else {
                        self.done = true;
                        Poll::Ready(Err(e))
                    }
                }
            }
        }
    }

    // -- RedisSet future --

    pub struct RedisSetFuture {
        id: u64,
        done: bool,
    }

    impl Future for RedisSetFuture {
        type Output = Result<(), String>;

        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done {
                panic!("polled after Ready")
            }
            match super::sys::async_resolve(self.id) {
                Ok(data) => {
                    self.done = true;
                    let resp = BoolResult::decode(data.as_slice()).unwrap_or_default();
                    if resp.success {
                        Poll::Ready(Ok(()))
                    } else {
                        Poll::Ready(Err(resp.error))
                    }
                }
                Err(e) => {
                    if e == "pending" {
                        Poll::Pending
                    } else {
                        self.done = true;
                        Poll::Ready(Err(e))
                    }
                }
            }
        }
    }
}

/// Async DB client.
pub mod db_async {
    use super::*;
    use crate::{DbQueryRequest, DbQueryResponse};

    /// A single database row returned by an async query.
    pub struct Row {
        pub fields: Vec<(String, String)>,
    }

    /// Submit a DB query to the goroutine pool.
    pub fn query(model: &str, operation: &str) -> DbFuture {
        let req = DbQueryRequest {
            model: model.into(),
            operation: operation.into(),
            conditions: None,
            fields: None,
            limit: 0,
            offset: 0,
        };
        DbFuture {
            id: super::sys::async_submit("db_query", &req.encode_to_vec()),
            done: false,
        }
    }

    pub struct DbFuture {
        id: u64,
        done: bool,
    }

    impl Future for DbFuture {
        type Output = Result<Vec<Row>, String>;

        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
            if self.done {
                panic!("polled after Ready")
            }
            match super::sys::async_resolve(self.id) {
                Ok(data) => {
                    self.done = true;
                    let resp = DbQueryResponse::decode(data.as_slice()).unwrap_or_default();
                    if !resp.error.is_empty() {
                        return Poll::Ready(Err(resp.error));
                    }
                    let rows: Vec<Row> = resp
                        .rows
                        .into_iter()
                        .map(|s| {
                            let fields: Vec<(String, String)> = s
                                .fields
                                .into_iter()
                                .map(|(k, v)| (k, format!("{:?}", v)))
                                .collect();
                            Row { fields }
                        })
                        .collect();
                    Poll::Ready(Ok(rows))
                }
                Err(e) => {
                    if e == "pending" {
                        Poll::Pending
                    } else {
                        self.done = true;
                        Poll::Ready(Err(e))
                    }
                }
            }
        }
    }
}

// Re-export the raw syscalls for advanced use.
pub mod sys {
    pub use crate::runtime::sys::{async_resolve, async_submit, async_wait_any};
}
