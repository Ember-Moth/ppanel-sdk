//! Event-loop async runtime backed by the host's goroutine pool.
//!
//! ## How it works
//!
//! 1. Guest calls `host::http::get(url)` which calls `sys::async_submit`.
//!    The host spawns a goroutine and returns a `task_id` immediately.
//! 2. The returned `HttpFuture` is `.await`ed.  On first poll it calls
//!    `sys::async_resolve(task_id)`.
//!    * If the host side is **blocking** (current behaviour) the call blocks
//!      the WASM thread until the goroutine finishes, so `poll` always returns
//!      `Ready`.
//!    * When the host becomes **non-blocking** `async_resolve` returns
//!      immediately with `done: false`, the future returns `Pending`, and
//!      `block_on` calls `sys::async_wait_any()` to yield the WASM thread
//!      until *any* goroutine completes.
//! 3. `block_on` re-polls.  Lather, rinse, repeat.
//!
//! ## Safety
//!
//! * A **pending-task counter** prevents `async_wait_any` from being called
//!   when there are no in-flight tasks (which would block forever).
//! * A **hard iteration cap** (10_000 polls) catches infinite-loop bugs in
//!   hand-rolled futures.

extern crate alloc;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

// ---------------------------------------------------------------------------
// no-op waker — WASM is single-threaded; the event loop re-polls externally
// ---------------------------------------------------------------------------

static WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(|d| RawWaker::new(d, &WAKER_VTABLE), |_| {}, |_| {}, |_| {});

fn noop_waker() -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &WAKER_VTABLE)) }
}

/// Safety limit: maximum number of poll iterations before we declare the
/// future stuck and return an error.
const MAX_POLL_ITERATIONS: u32 = 10_000;

// ---------------------------------------------------------------------------
// Pending-task counter
// ---------------------------------------------------------------------------
// Incremented by `sys::async_submit`, decremented by `sys::async_resolve`
// (when the task actually completes).  `block_on` reads it to decide whether
// it is safe to call `async_wait_any`.

static mut PENDING_COUNT: u32 = 0;

/// Number of async tasks submitted but not yet resolved.
pub fn pending_count() -> u32 {
    unsafe { PENDING_COUNT }
}

fn inc_pending() {
    unsafe {
        PENDING_COUNT = PENDING_COUNT.saturating_add(1);
    }
}

fn dec_pending() {
    unsafe {
        PENDING_COUNT = PENDING_COUNT.saturating_sub(1);
    }
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

/// Drive a future to completion.
///
/// Returns `Err("future stalled: …")` when the future returns `Pending` but
/// no async tasks are in-flight — there is nothing that could ever wake it.
///
/// Returns `Err("future exceeded max poll iterations")` when the hard safety
/// cap is hit (indicating a buggy future that never becomes ready).
pub fn block_on<F: Future>(mut fut: F) -> Result<F::Output, &'static str> {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    // SAFETY: the future is pinned on this stack frame and never moved.
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };

    for _iteration in 0..MAX_POLL_ITERATIONS {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(r) => return Ok(r),
            Poll::Pending => {
                // If nothing is in flight the future will never make progress.
                if pending_count() == 0 {
                    return Err("future stalled: returned Pending with no async tasks submitted");
                }
                // Yield the WASM thread until any goroutine finishes.
                sys::async_wait_any();
            }
        }
    }

    Err("future exceeded max poll iterations — probable infinite loop")
}

// ---------------------------------------------------------------------------
// Raw syscalls to the Go host
// ---------------------------------------------------------------------------

pub mod sys {
    use prost::Message;

    use crate::abi;

    // Host imports — (i32, i32) -> i64
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn host_async_submit(ptr: i32, len: i32) -> i64;
        fn host_async_resolve(ptr: i32, len: i32) -> i64;
        fn host_async_wait_any(ptr: i32, len: i32) -> i64;
    }

    /// Call any host async function: encode → write WASM memory → call → read.
    unsafe fn call_host(
        f: unsafe extern "C" fn(i32, i32) -> i64,
        req: &[u8],
    ) -> alloc::vec::Vec<u8> {
        let packed = abi::encode_bytes(req);
        if packed == 0 && req.len() > 0 {
            return alloc::vec![]; // allocation failed
        }
        let p = (packed >> 32) as i32;
        let l = (packed & 0xFFFF_FFFF) as i32;
        let rp = f(p, l);

        abi::deallocate(p, l);

        let rptr = (rp >> 32) as u32;
        let rlen = (rp & 0xFFFF_FFFF) as u32;
        if rlen == 0 {
            return alloc::vec![];
        }

        let vec = core::slice::from_raw_parts(rptr as *const u8, rlen as usize).to_vec();
        abi::deallocate(rptr as i32, rlen as i32);
        vec
    }

    /// Submit an async task to a Go goroutine.  Returns `task_id` immediately.
    /// Increments the pending-task counter.
    pub fn async_submit(op: &str, params: &[u8]) -> u64 {
        use crate::{AsyncSubmitRequest, AsyncSubmitResponse};

        let req = AsyncSubmitRequest {
            op_type: op.into(),
            params: params.to_vec(),
        };
        let rb = unsafe { call_host(host_async_submit, &req.encode_to_vec()) };
        let resp = AsyncSubmitResponse::decode(rb.as_slice()).unwrap_or_default();
        if resp.task_id != 0 {
            super::inc_pending();
        }
        resp.task_id
    }

    /// Resolve a previously submitted async task.
    ///
    /// **Current host behaviour (blocking):** blocks the WASM thread until the
    /// goroutine finishes, then returns `Ok(result)`.  Callers that need the
    /// value *now* can call this directly; `poll` will always get `Ready`.
    ///
    /// **Planned non-blocking behaviour:** returns immediately.  `Ok(data)`
    /// when the task is done, `Err("pending")` when it is still running.
    /// Decrements the pending-task counter on completion.
    pub fn async_resolve(task_id: u64) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        use crate::{AsyncResolveRequest, AsyncResolveResponse};

        let req = AsyncResolveRequest { task_id };
        let rb = unsafe { call_host(host_async_resolve, &req.encode_to_vec()) };
        let resp = AsyncResolveResponse::decode(rb.as_slice()).unwrap_or_default();

        if resp.done {
            super::dec_pending();
            if resp.error.is_empty() {
                Ok(resp.result)
            } else {
                Err(resp.error)
            }
        } else {
            // Host returned done=false — task is still running.
            // The counter stays incremented; we will retry on the next poll.
            Err("pending".into())
        }
    }

    /// Block until *any* pending async task completes.  Returns its `task_id`.
    ///
    /// # Safety
    ///
    /// Callers MUST ensure `pending_count() > 0` before calling this, otherwise
    /// it will block the WASM thread forever (the Go channel will never receive).
    pub fn async_wait_any() -> u64 {
        use crate::{AsyncWaitAnyRequest, AsyncWaitAnyResponse};

        let req = AsyncWaitAnyRequest {};
        let rb = unsafe { call_host(host_async_wait_any, &req.encode_to_vec()) };
        AsyncWaitAnyResponse::decode(rb.as_slice())
            .map(|r| r.task_id)
            .unwrap_or(0)
    }
}
