//! PPanel SDK 教学插件。
//!
//! 这个示例刻意覆盖插件开发最常见的路径：
//! - 在 init 中注册 HTTP 路由和自定义中间件。
//! - 读取请求 query、header、body 和用户上下文。
//! - 通过 host function 访问配置、Redis、数据库和事件系统。
//! - 使用 async handler 把耗时宿主操作交给 Go goroutine 池执行。
//! - 用 `export = "..."`
//!   显式声明 WASM 导出名，保证 server 注册名和 Rust 函数名可以独立演进。

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use ppanel_sdk::prelude::*;
use ppanel_sdk::prost_types::{value::Kind, Struct, Value};

const DEMO_TOKEN: &str = "let-me-in";

#[ppanel_sdk::init]
fn init(req: InitRequest) -> Result<(), String> {
    let host = req.host_config.unwrap_or_default();
    let greeting = req
        .config
        .as_ref()
        .and_then(|config| config.fields.get("greeting"))
        .and_then(value_to_string)
        .unwrap_or_else(|| "你好".to_string());

    host::log::info(&format!(
        "demo-plugin init: site={}, greeting={}",
        host.site_name, greeting
    ));

    host::middleware::register("demo_guard", "mw_demo_guard")?;
    host::route::register("GET", "/hello", "handle_hello")?;
    host::route::register("POST", "/echo", "handle_echo")?;
    host::route::register("GET", "/redis/counter", "handle_redis_counter")?;
    host::route::register("GET", "/async/redis", "handle_async_redis")?;
    host::route::register("GET", "/db/users", "handle_db_users")?;
    host::route::register_with_middleware("GET", "/guarded", "demo_guard", "handle_guarded")?;
    host::events::subscribe("demo.ping", "on_demo_ping")?;

    Ok(())
}

#[ppanel_sdk::handler(export = "handle_hello")]
fn hello(req: HandleRequest) -> HandleResponse {
    let name = first_query(&req, "name").unwrap_or_else(|| "开发者".to_string());
    let site = host::config::get("Site.SiteName").unwrap_or_else(|_| "PPanel".to_string());
    let user_id = req
        .context
        .as_ref()
        .map(|ctx| ctx.user_id)
        .unwrap_or_default();

    json_response(
        200,
        format!(
            r#"{{"message":"你好，{}","site":"{}","user_id":{}}}"#,
            escape_json(&name),
            escape_json(&site),
            user_id
        ),
    )
}

#[ppanel_sdk::handler(export = "handle_echo")]
fn echo(req: HandleRequest) -> HandleResponse {
    let body = String::from_utf8(req.body).unwrap_or_else(|_| "<non-utf8 body>".to_string());
    json_response(
        200,
        format!(
            r#"{{"method":"{}","path":"{}","body":"{}"}}"#,
            escape_json(&req.method),
            escape_json(&req.path),
            escape_json(&body)
        ),
    )
}

#[ppanel_sdk::handler(export = "handle_redis_counter")]
fn redis_counter(_req: HandleRequest) -> HandleResponse {
    let current = host::redis::get("counter")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let next = current + 1;

    match host::redis::set("counter", &next.to_string(), 3600) {
        Ok(_) => json_response(200, format!(r#"{{"counter":{}}}"#, next)),
        Err(err) => json_response(500, format!(r#"{{"error":"{}"}}"#, escape_json(&err))),
    }
}

#[ppanel_sdk::handler(export = "handle_async_redis")]
async fn async_redis(_req: HandleRequest) -> HandleResponse {
    if let Err(err) = host::redis_async::set("async_message", "hello from async", 3600).await {
        return json_response(500, format!(r#"{{"error":"{}"}}"#, escape_json(&err)));
    }

    match host::redis_async::get("async_message").await {
        Ok(value) => json_response(
            200,
            format!(r#"{{"async":true,"value":"{}"}}"#, escape_json(&value)),
        ),
        Err(err) => json_response(500, format!(r#"{{"error":"{}"}}"#, escape_json(&err))),
    }
}

#[ppanel_sdk::handler(export = "handle_db_users")]
fn db_users(_req: HandleRequest) -> HandleResponse {
    let mut fields = BTreeMap::new();
    fields.insert(
        "status".to_string(),
        Value {
            kind: Some(Kind::StringValue("active".to_string())),
        },
    );

    let db_req = DbQueryRequest {
        model: "user".to_string(),
        operation: "find".to_string(),
        conditions: Some(Struct { fields }),
        fields: None,
        limit: 10,
        offset: 0,
    };

    match host::db_sync::query_raw(&db_req) {
        Ok(resp) if resp.error.is_empty() => json_response(
            200,
            format!(r#"{{"rows":{},"total":{}}}"#, resp.rows.len(), resp.total),
        ),
        Ok(resp) => json_response(
            500,
            format!(r#"{{"error":"{}"}}"#, escape_json(&resp.error)),
        ),
        Err(err) => json_response(500, format!(r#"{{"error":"{}"}}"#, escape_json(&err))),
    }
}

#[ppanel_sdk::handler(export = "handle_guarded")]
fn guarded(req: HandleRequest) -> HandleResponse {
    let user_agent = first_header(&req, "User-Agent").unwrap_or_else(|| "unknown".to_string());
    json_response(
        200,
        format!(
            r#"{{"ok":true,"user_agent":"{}"}}"#,
            escape_json(&user_agent)
        ),
    )
}

#[ppanel_sdk::middleware(export = "mw_demo_guard")]
fn demo_guard(req: HandleRequest) -> MiddlewareResponse {
    if first_header(&req, "X-Demo-Token").as_deref() != Some(DEMO_TOKEN) {
        return MiddlewareResponse {
            action: "abort".to_string(),
            status: 403,
            headers: json_headers(),
            body: br#"{"error":"missing or invalid X-Demo-Token"}"#.to_vec(),
        };
    }

    let mut headers = BTreeMap::new();
    headers.insert("x-demo-guard".to_string(), "passed".to_string());
    MiddlewareResponse {
        action: "next".to_string(),
        status: 200,
        headers,
        body: vec![],
    }
}

#[ppanel_sdk::event_handler(export = "on_demo_ping")]
fn on_demo_ping(req: EmitEventRequest) -> BoolResult {
    let payload = req
        .payload
        .as_ref()
        .and_then(|payload| payload.fields.get("message"))
        .and_then(value_to_string)
        .unwrap_or_else(|| "empty".to_string());

    match host::redis::set("last_event", &payload, 3600) {
        Ok(_) => BoolResult {
            success: true,
            error: String::new(),
        },
        Err(err) => BoolResult {
            success: false,
            error: err,
        },
    }
}

fn json_response(status: i32, body: String) -> HandleResponse {
    HandleResponse {
        status,
        headers: json_headers(),
        body: body.into_bytes(),
    }
}

fn json_headers() -> BTreeMap<String, String> {
    BTreeMap::from([("Content-Type".to_string(), "application/json".to_string())])
}

fn first_query(req: &HandleRequest, key: &str) -> Option<String> {
    req.query
        .get(key)
        .and_then(|list| list.values.first())
        .cloned()
}

fn first_header(req: &HandleRequest, key: &str) -> Option<String> {
    req.headers
        .get(key)
        .or_else(|| req.headers.get(&key.to_ascii_lowercase()))
        .and_then(|list| list.values.first())
        .cloned()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value.kind.as_ref()? {
        Kind::StringValue(value) => Some(value.clone()),
        Kind::NumberValue(value) => Some(value.to_string()),
        Kind::BoolValue(value) => Some(value.to_string()),
        _ => None,
    }
}

fn escape_json(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
