//! Low-level WASM ABI — memory management and protobuf encoding.
use prost::Message;

static mut BUFFER: [u8; 65536] = [0u8; 65536];
static mut BUMP_OFFSET: usize = 0;

#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    let size = size as usize;
    unsafe {
        let ptr = BUFFER.as_ptr().add(BUMP_OFFSET) as usize;
        BUMP_OFFSET += size;
        if BUMP_OFFSET > BUFFER.len() { BUMP_OFFSET = size; }
        ptr as i32
    }
}

#[no_mangle] pub extern "C" fn deallocate(_ptr: i32, _size: i32) {}

pub fn encode_bytes(data: &[u8]) -> i64 {
    let len = data.len() as i32;
    if len == 0 { return 0; }
    let ptr = allocate(len);
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, len as usize); }
    ((ptr as i64) << 32) | (len as i64)
}

pub fn encode_response<T: Message>(msg: &T) -> i64 { encode_bytes(&msg.encode_to_vec()) }
