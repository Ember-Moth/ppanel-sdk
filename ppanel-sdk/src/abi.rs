//! Low-level WASM ABI — memory management and protobuf encoding.

use alloc::alloc::{alloc, dealloc, Layout};
use prost::Message;

/// Reset the bump allocator. (No-op in dynamic allocator)
pub fn reset() {}

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    let size = size as usize;
    if size == 0 {
        return 0;
    }
    let layout = Layout::from_size_align(size, 8).unwrap();
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        return 0;
    }
    ptr as i32
}

#[no_mangle]
pub extern "C" fn deallocate(ptr: i32, size: i32) {
    if ptr == 0 || size == 0 {
        return;
    }
    let layout = Layout::from_size_align(size as usize, 8).unwrap();
    unsafe {
        dealloc(ptr as *mut u8, layout);
    }
}

pub fn encode_bytes(data: &[u8]) -> i64 {
    let len = data.len();
    if len == 0 {
        return 0;
    }
    let len_i32 = len as i32;
    let ptr = allocate(len_i32);
    if ptr == 0 {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, len);
    }
    (((ptr as u32) as u64) << 32 | ((len_i32 as u32) as u64)) as i64
}

pub fn encode_response<T: Message>(msg: &T) -> i64 {
    encode_bytes(&msg.encode_to_vec())
}
