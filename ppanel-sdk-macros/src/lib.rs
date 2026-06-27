//! PPanel process macros. Supports sync and `async fn`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

fn is_async(f: &ItemFn) -> bool { f.sig.asyncness.is_some() }

#[proc_macro_attribute]
pub fn init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let call = if is_async(&input) { quote! { ppanel_sdk::runtime::block_on(init(req)) } } else { quote! { init(req) } };
    (quote! {
        #input
        #[export_name = "init"]
        pub extern "C" fn __ppanel_init_wasm(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi; use ppanel_sdk::prost::Message as _;
            let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let req = match ppanel_sdk::InitRequest::decode(bytes) { Ok(r) => r, Err(e) => return abi::encode_response(&ppanel_sdk::BoolResult { success: false, error: format!("decode: {}", e) }) };
            abi::encode_response(&match #call { Ok(_) => ppanel_sdk::BoolResult { success: true, error: String::new() }, Err(e) => ppanel_sdk::BoolResult { success: false, error: e } })
        }
    }).into()
}

#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let wasm_name = quote::format_ident!("__ppanel_handler_{}_wasm", fn_name);
    let export_str = format!("handle_{}", fn_name);
    let call = if is_async(&input) { quote! { ppanel_sdk::runtime::block_on(#fn_name(req)) } } else { quote! { #fn_name(req) } };
    (quote! {
        #input
        #[export_name = #export_str]
        pub extern "C" fn #wasm_name(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi; use ppanel_sdk::prost::Message as _;
            let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
            let req = match ppanel_sdk::HandleRequest::decode(bytes) { Ok(r) => r, Err(e) => return abi::encode_response(&ppanel_sdk::HandleResponse { status: 500, body: format!("{{\"error\":\"{}\"}}", e).into_bytes(), headers: std::collections::HashMap::new() }) };
            abi::encode_response(&#call)
        }
    }).into()
}
