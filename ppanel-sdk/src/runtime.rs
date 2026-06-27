//! Event-loop async runtime. Goroutines as thread pool.

extern crate alloc;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

static WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(|d| RawWaker::new(d, &WAKER_VTABLE), |_| {}, |_| {}, |_| {});
fn noop_waker() -> Waker { unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &WAKER_VTABLE)) } }

/// Drive a future to completion via event loop.
/// On Pending, blocks on host_async_wait_any until a goroutine completes.
pub fn block_on<F: Future>(mut fut: F) -> F::Output {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(r) => return r,
            Poll::Pending => { sys::async_wait_any(); }
        }
    }
}

/// Raw syscalls to Go host async functions.
pub mod sys {
    use prost::Message;
    use crate::abi;

    extern "C" {
        fn host_async_submit(ptr: i32, len: i32) -> i64;
        fn host_async_resolve(ptr: i32, len: i32) -> i64;
        fn host_async_wait_any(ptr: i32, len: i32) -> i64;
    }

    unsafe fn call_host(f: unsafe extern "C" fn(i32, i32) -> i64, req: &[u8]) -> alloc::vec::Vec<u8> {
        let packed = abi::encode_bytes(req);
        let p = (packed >> 32) as i32; let l = (packed & 0xFFFFFFFF) as i32;
        let rp = f(p, l); let rptr = (rp >> 32) as u32; let rlen = (rp & 0xFFFFFFFF) as u32;
        if rlen == 0 { return alloc::vec![]; }
        core::slice::from_raw_parts(rptr as *const u8, rlen as usize).to_vec()
    }

    /// Submit an async task to a Go goroutine. Returns task_id immediately.
    pub fn async_submit(op: &str, params: &[u8]) -> u64 {
        use crate::{AsyncSubmitRequest, AsyncSubmitResponse};
        let req = AsyncSubmitRequest { op_type: op.into(), params: params.to_vec() };
        let rb = unsafe { call_host(host_async_submit, &req.encode_to_vec()) };
        AsyncSubmitResponse::decode(rb.as_slice()).unwrap().task_id
    }

    /// Block until the specified task completes. Returns result.
    pub fn async_resolve(task_id: u64) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        use crate::{AsyncResolveRequest, AsyncResolveResponse};
        let req = AsyncResolveRequest { task_id };
        let rb = unsafe { call_host(host_async_resolve, &req.encode_to_vec()) };
        let resp = AsyncResolveResponse::decode(rb.as_slice()).unwrap();
        if resp.error.is_empty() { Ok(resp.result) } else { Err(resp.error) }
    }

    /// Block until any pending async task completes. Returns its task_id.
    pub fn async_wait_any() -> u64 {
        use crate::{AsyncWaitAnyRequest, AsyncWaitAnyResponse};
        let req = AsyncWaitAnyRequest {};
        let rb = unsafe { call_host(host_async_wait_any, &req.encode_to_vec()) };
        AsyncWaitAnyResponse::decode(rb.as_slice()).unwrap().task_id
    }
}
