# ppanel-sdk 插件开发教程

`ppanel-sdk` 是 PPanel 插件系统的 Rust SDK。插件会被编译成
`wasm32-wasip1` 目标的 WASM 文件，由 server 侧的 WASI runtime 加载，并通过
`env.*` host functions 调用宿主能力。

本文档以 `examples/demo-plugin` 为教学示例，演示一个插件从编写、构建到部署的完整流程。

## 1. 插件运行模型

一个插件由两部分组成：

- `plugin.yaml`：插件清单，声明名称、版本、入口 WASM 文件和权限。
- `plugin.wasm`：Rust 编译出的 WASI preview1 WASM 模块。

server 启动后会扫描插件目录，读取 `plugin.yaml`，按权限注入 host functions，
然后调用插件的 `init` 和可选 `start`。插件通常在 `init` 中注册路由、中间件、
事件订阅和定时任务。

HTTP 请求的最终路径格式是：

```text
/v1/plugin/{plugin_name}/{plugin_path}
```

例如 demo 插件注册了 `/hello`，访问地址就是：

```text
GET /v1/plugin/demo-plugin/hello?name=Alice
```

## 2. 准备环境

安装 Rust WASI target：

```bash
rustup target add wasm32-wasip1
```

确认 SDK workspace 可以编译：

```bash
cargo check --workspace
```

## 3. 教学 demo 目录结构

当前 examples 目录只保留一个教学插件：

```text
examples/demo-plugin/
  .cargo/config.toml
  Cargo.toml
  plugin.yaml
  src/lib.rs
```

`.cargo/config.toml` 固定构建目标：

```toml
[build]
target = "wasm32-wasip1"
```

`Cargo.toml` 需要声明 `cdylib`：

```toml
[lib]
crate-type = ["cdylib"]
```

## 4. 编写 plugin.yaml

demo 插件的清单如下：

```yaml
name: demo-plugin
version: 0.1.0
description: PPanel SDK 教学插件，演示路由、中间件、Redis、DB、事件、配置读取和异步 host 调用
author: PPanel
main: plugin.wasm
permissions:
  - http_routes
  - middleware
  - redis
  - database_read
  - logging
  - config_read
  - events
config:
  greeting: 你好
```

常用权限说明：

| 权限 | 能力 |
| --- | --- |
| `http_routes` | 允许调用 `host::route::register` 注册 HTTP 路由 |
| `middleware` | 允许注册 WASM 自定义中间件 |
| `redis` | 允许访问插件隔离命名空间下的 Redis key |
| `database_read` | 允许通过白名单模型读取数据库 |
| `database_write` | 允许通过白名单模型写数据库 |
| `logging` | 允许写宿主日志 |
| `config_read` | 允许读取安全白名单内的宿主配置 |
| `events` | 允许发布和订阅宿主事件 |
| `http_client` | 允许发起带 SSRF 防护的外部 HTTP 请求 |
| `scheduler` | 允许注册 cron 定时任务 |
| `queue` | 允许把任务投递到宿主队列 |

只声明实际需要的权限。未声明权限时，对应 host function 不会注入，插件调用会失败。

## 5. 插件入口 init

`init` 是插件初始化入口，推荐只做注册类工作：

```rust
#[ppanel_sdk::init]
fn init(req: InitRequest) -> Result<(), String> {
    let host = req.host_config.unwrap_or_default();
    host::log::info(&format!("demo-plugin init: site={}", host.site_name));

    host::middleware::register("demo_guard", "mw_demo_guard")?;
    host::route::register("GET", "/hello", "handle_hello")?;
    host::route::register("POST", "/echo", "handle_echo")?;
    host::route::register("GET", "/redis/counter", "handle_redis_counter")?;
    host::route::register("GET", "/async/redis", "handle_async_redis")?;
    host::route::register("GET", "/db/users", "handle_db_users")?;
    host::route::register_with_middleware(
        "GET",
        "/guarded",
        "demo_guard",
        "handle_guarded",
    )?;
    host::events::subscribe("demo.ping", "on_demo_ping")?;
    Ok(())
}
```

注意：

- `init` 不接收 HTTP 请求，它接收 `InitRequest`。
- 路由 handler 名称必须匹配对应 Rust handler 的 `export` 名。
- 当前 server 默认每个插件单 WASM 实例运行，请避免在 `init/start` 中写入不可重复的副作用。

## 6. 编写 HTTP Handler

Handler 接收 `HandleRequest`，返回 `HandleResponse`。

```rust
#[ppanel_sdk::handler(export = "handle_hello")]
fn hello(req: HandleRequest) -> HandleResponse {
    let name = req
        .query
        .get("name")
        .and_then(|list| list.values.first())
        .cloned()
        .unwrap_or_else(|| "开发者".to_string());

    HandleResponse {
        status: 200,
        headers: BTreeMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
        ]),
        body: format!(r#"{{"message":"你好，{}"}}"#, name).into_bytes(),
    }
}
```

`HandleRequest` 包含：

- `method`：HTTP method。
- `path`：请求路径。
- `query`：query string，多值结构。
- `headers`：请求头，多值结构。
- `body`：请求体 bytes。
- `context`：宿主注入的上下文，例如 `user_id`、`is_admin`、`client_ip`。

## 7. 自定义中间件

demo 插件的 `demo_guard` 中间件要求请求带上：

```text
X-Demo-Token: let-me-in
```

中间件返回值的 `action` 有三个常用值：

- `next`：继续执行 handler。
- `abort`：中断请求并直接返回响应。
- `modify`：修改请求 header 后继续。

```rust
#[ppanel_sdk::middleware(export = "mw_demo_guard")]
fn demo_guard(req: HandleRequest) -> MiddlewareResponse {
    if first_header(&req, "X-Demo-Token").as_deref() != Some("let-me-in") {
        return MiddlewareResponse {
            action: "abort".to_string(),
            status: 403,
            headers: json_headers(),
            body: br#"{"error":"missing or invalid X-Demo-Token"}"#.to_vec(),
        };
    }

    MiddlewareResponse {
        action: "next".to_string(),
        status: 200,
        headers: BTreeMap::from([
            ("x-demo-guard".to_string(), "passed".to_string()),
        ]),
        body: vec![],
    }
}
```

## 8. Redis、DB 和事件

Redis key 会自动加上插件名前缀，插件侧只需要传业务 key：

```rust
let current = host::redis::get("counter").unwrap_or_else(|_| "0".to_string());
host::redis::set("counter", "1", 3600)?;
```

数据库查询通过白名单模型和字段执行：

```rust
let db_req = DbQueryRequest {
    model: "user".to_string(),
    operation: "find".to_string(),
    conditions: Some(condition_struct),
    fields: None,
    limit: 10,
    offset: 0,
};
let resp = host::db_sync::query_raw(&db_req)?;
```

事件订阅在 `init` 中注册：

```rust
host::events::subscribe("demo.ping", "on_demo_ping")?;
```

事件处理函数使用 `#[event_handler]`：

```rust
#[ppanel_sdk::event_handler(export = "on_demo_ping")]
fn on_demo_ping(req: EmitEventRequest) -> BoolResult {
    host::log::info(&format!("event={}", req.event));
    BoolResult { success: true, error: String::new() }
}
```

## 9. 异步 Runtime 和 async handler

插件运行在 WASM 内，不能直接创建 Go goroutine。我们的异步设计是：**插件侧写
`async fn`，SDK 把宿主 I/O 提交给 server，server 用受限 goroutine 执行 Redis、DB
或 HTTP 操作，完成后再把 protobuf 结果返回给 WASM**。

### 9.1 为什么需要这层异步

同步 host function 会在当前 WASM 调用里等待宿主返回。如果一个 handler 直接做慢 HTTP
请求或慢 DB 查询，这个插件实例会一直被占用。异步 host API 把慢 I/O 移到 server 侧
goroutine 执行，并用 `Future` 给插件开发者保留普通 Rust async/await 写法。

当前 server 默认每个插件一个 WASM 实例，所以同一插件的 handler 仍然是串行进入 WASM。
异步的价值不是让同一个 WASM 实例并行跑多个 handler，而是：

- 宿主 I/O 不在 WASM 调用栈里直接执行。
- 多个互不依赖的 I/O 可以先 submit，再 await，让 server 先收到多个任务。
- ABI 已经支持后续把 `resolve` 改成非阻塞调度，不需要插件代码重写。

### 9.2 ABI 三个 host function

SDK 和 server 之间的异步 ABI 是三组 `env.*` host imports：

| Host function | 请求 | 响应 | 作用 |
| --- | --- | --- | --- |
| `host_async_submit` | `AsyncSubmitRequest` | `AsyncSubmitResponse` | 提交任务，返回 `task_id` |
| `host_async_resolve` | `AsyncResolveRequest` | `AsyncResolveResponse` | 获取指定任务结果 |
| `host_async_wait_any` | `AsyncWaitAnyRequest` | `AsyncWaitAnyResponse` | 等任意任务完成 |

`AsyncSubmitRequest` 里最关键的是：

- `op_type`：任务类型，目前包括 `redis_get`、`redis_set`、`db_query`、`http_get`、`http_post`。
- `params`：对应同步请求消息的 protobuf bytes，例如 `RedisGetRequest`、`DbQueryRequest` 或
  `HttpRequestRequest`。

`AsyncResolveResponse` 里最关键的是：

- `done`：任务是否完成。
- `result`：对应响应消息的 protobuf bytes。
- `error`：宿主执行错误或权限错误。

### 9.3 SDK 如何把 async fn 跑起来

当你写：

```rust
#[ppanel_sdk::handler(export = "handle_async_redis")]
async fn async_redis(_req: HandleRequest) -> HandleResponse {
    if let Err(err) = host::redis_async::set("async_message", "hello from async", 3600).await {
        return json_response(500, format!(r#"{{"error":"{}"}}"#, err));
    }

    json_response(200, r#"{"ok":true}"#.to_string())
}
```

`#[handler]` 宏会生成真实导出的 WASM 函数 `handle_async_redis`。这个导出函数仍然是普通
`extern "C"` 函数，server 可以按同步 ABI 调它；导出函数内部会调用
`ppanel_sdk::runtime::block_on(async_redis(req))` 驱动 future。

`host::redis_async::set(...)` 本身不会直接访问 Redis，它会：

1. 编码 `RedisSetRequest`。
2. 调用 `host_async_submit("redis_set", bytes)` 拿到 `task_id`。
3. 返回 `RedisSetFuture`。
4. `.await` 时调用 `host_async_resolve(task_id)`。
5. 解码 `BoolResult`，转成 `Result<(), String>`。

当前 server 实现里，`host_async_resolve` 会阻塞到这个任务完成，因此 future 第一次 poll
通常就会 `Ready`。如果以后 server 改成非阻塞 resolve，当任务未完成时会返回 `done=false`，
SDK future 会返回 `Poll::Pending`，`runtime::block_on` 会调用 `host_async_wait_any()` 等任意
任务完成后再重新 poll。

### 9.4 server 侧如何执行和限流

server 为每个插件维护异步任务表和通知队列：

- `submitAsyncTask` 创建 task，分配递增 `task_id`。
- `executeAsyncTask` 在 goroutine 中执行真实 Redis、DB 或 HTTP 操作。
- `resolveAsyncTask` 根据 `task_id` 取回结果，并在完成后清理任务。
- `waitAnyAsyncTask` 从插件自己的完成通知队列里取一个已完成任务 id。

每个插件还有独立 in-flight 限流，当前默认最多 64 个未完成 async task。超过限制时
`host_async_submit` 返回 `task_id = 0`，SDK resolve 时会得到 `task not found` 这类错误，
避免单个插件无限制创建 goroutine。

异步任务仍然会检查权限：

- `redis_get` / `redis_set` 需要 `redis`。
- `db_query` 读操作需要 `database_read` 或 `database_write`，写操作需要 `database_write`。
- `http_get` / `http_post` 需要 `http_client`，并继续走 server 的 URL 安全检查。

demo 插件里的异步路由：

```rust
host::route::register("GET", "/async/redis", "handle_async_redis")?;
```

对应 handler：

```rust
#[ppanel_sdk::handler(export = "handle_async_redis")]
async fn async_redis(_req: HandleRequest) -> HandleResponse {
    if let Err(err) = host::redis_async::set("async_message", "hello from async", 3600).await {
        return json_response(
            500,
            format!(r#"{{"error":"{}"}}"#, escape_json(&err)),
        );
    }

    match host::redis_async::get("async_message").await {
        Ok(value) => json_response(
            200,
            format!(r#"{{"async":true,"value":"{}"}}"#, escape_json(&value)),
        ),
        Err(err) => json_response(500, format!(r#"{{"error":"{}"}}"#, escape_json(&err))),
    }
}
```

使用建议：

- 适合 Redis、DB、HTTP 这类宿主 I/O。
- 不适合 CPU 密集计算；CPU 逻辑仍然在当前 WASM 调用里执行。
- 多个互不依赖的 async 操作可以先创建 future，再分别 await，让 server 先收到多个任务。
- 如果 async API 返回错误，不要 panic，应该转成明确 HTTP 响应或插件日志，方便 server 侧定位。

## 10. 构建 demo 插件

在 `ppanel-sdk` 根目录运行：

```bash
cargo build -p demo-plugin --target wasm32-wasip1 --release
```

产物位置：

```text
target/wasm32-wasip1/release/demo_plugin.wasm
```

## 11. 部署到 server

server 默认插件目录配置是 `plugins`，这是相对 server 进程工作目录解析的路径，不是自动相对二进制文件所在目录。
也可以在配置中把 `Plugin.Directory` 改成绝对路径。

假设当前工作目录是 `/root/project-moth/server`，插件目录就是 `/root/project-moth/server/plugins`。
构建产物在相邻的 `../ppanel-sdk` 目录：

```bash
mkdir -p plugins/demo-plugin
cp ../ppanel-sdk/examples/demo-plugin/plugin.yaml plugins/demo-plugin/plugin.yaml
cp ../ppanel-sdk/target/wasm32-wasip1/release/demo_plugin.wasm plugins/demo-plugin/plugin.wasm
```

启动 server 后可以访问：

```bash
curl 'http://127.0.0.1:8080/v1/plugin/demo-plugin/hello?name=Alice'
curl -X POST 'http://127.0.0.1:8080/v1/plugin/demo-plugin/echo' -d 'hello'
curl 'http://127.0.0.1:8080/v1/plugin/demo-plugin/redis/counter'
curl 'http://127.0.0.1:8080/v1/plugin/demo-plugin/async/redis'
curl -H 'X-Demo-Token: let-me-in' \
  'http://127.0.0.1:8080/v1/plugin/demo-plugin/guarded'
```

## 12. 常见问题

**handler 找不到**

确认注册名和导出名一致：

```rust
host::route::register("GET", "/hello", "handle_hello")?;

#[ppanel_sdk::handler(export = "handle_hello")]
fn hello(req: HandleRequest) -> HandleResponse { ... }
```

**权限错误**

确认 `plugin.yaml` 里声明了对应权限。例如使用 Redis 必须声明：

```yaml
permissions:
  - redis
```

**目标 ABI 错误**

当前插件必须用 `wasm32-wasip1`，不要使用 `wasm32-unknown-unknown`。
server 会实例化 WASI preview1，并提供 stdout/stderr、随机数、时间和 `/data` 挂载。

**并发请求如何处理**

当前 server 默认每个插件一个 WASM 实例，所以同一插件的 handler 调用会排队执行。
耗时 I/O 尽量使用 SDK 的 async host API，把外部操作交给宿主 goroutine 池执行。详细机制见
“异步 Runtime 和 async handler”章节。

**如何调试**

- 在插件中使用 `host::log::info/error` 写日志。
- `println!` 会输出到 WASI stdout。
- 先运行 `cargo check --workspace`，再构建 WASM。
- 改动 ABI 或 proto 后，先确保 server 和 SDK 两侧消息定义一致。
