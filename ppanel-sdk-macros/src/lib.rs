//! PPanel procedural macros.
//!
//! | Macro | WASM export | Signature |
//! |-------|------------|-----------|
//! | `#[ppanel_sdk::init]` | `init` | `fn(InitRequest) -> Result<(), String>` |
//! | `#[ppanel_sdk::handler]` | `handle_<fn_name>` | `fn(HandleRequest) -> HandleResponse` |
//! | `#[ppanel_sdk::start]` | `start` | `fn() -> Result<(), String>` |
//! | `#[ppanel_sdk::stop]` | `stop` | `fn() -> Result<(), String>` |
//! | `#[ppanel_sdk::event_handler]` | `on_<fn_name>` | `fn(EmitEventRequest) -> BoolResult` |

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Expr, ItemFn, Lit, MetaNameValue, Token};

fn is_async(f: &ItemFn) -> bool {
    f.sig.asyncness.is_some()
}

fn compile_error(msg: &str) -> TokenStream {
    quote! { compile_error!(#msg); }.into()
}

fn ensure_no_attr(attr: TokenStream, macro_name: &str) -> Result<(), TokenStream> {
    if attr.is_empty() {
        Ok(())
    } else {
        Err(compile_error(&format!(
            "#[ppanel_sdk::{}] does not accept attributes",
            macro_name
        )))
    }
}

fn parse_export_attr(
    attr: TokenStream,
    default_export: String,
    macro_name: &str,
) -> Result<String, TokenStream> {
    if attr.is_empty() {
        return Ok(default_export);
    }

    let parser = Punctuated::<MetaNameValue, Token![,]>::parse_terminated;
    let pairs = parser.parse(attr).map_err(|err| {
        compile_error(&format!(
            "invalid #[ppanel_sdk::{}] attribute: {}",
            macro_name, err
        ))
    })?;

    let mut export = default_export;
    for pair in pairs {
        let Some(ident) = pair.path.get_ident() else {
            return Err(compile_error("attribute key must be an identifier"));
        };
        let key = ident.to_string();
        if key != "export" {
            return Err(compile_error(&format!(
                "unsupported #[ppanel_sdk::{}({} = ...)] attribute; use export = \"wasm_export_name\"",
                macro_name, key
            )));
        }
        match pair.value {
            Expr::Lit(expr_lit) => match expr_lit.lit {
                Lit::Str(s) => export = s.value(),
                _ => return Err(compile_error("export must be a string literal")),
            },
            _ => return Err(compile_error("export must be a string literal")),
        }
    }
    Ok(export)
}

// =========================================================================
// #[init] — wraps a fn(InitRequest) -> Result<(), String>
// =========================================================================

#[proc_macro_attribute]
pub fn init(attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Err(err) = ensure_no_attr(attr, "init") {
        return err;
    }
    let input = parse_macro_input!(item as ItemFn);
    let is_async = is_async(&input);
    let fn_name = &input.sig.ident;

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name(req)) {
                Ok(result) => result,
                Err(block_err) => Err(alloc::format!("block_on: {}", block_err)),
            }
        }
    } else {
        quote! { #fn_name(req) }
    };

    (quote! {
        #input

        #[export_name = "init"]
        pub extern "C" fn __ppanel_init_wasm(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi;
            use ppanel_sdk::prost::Message as _;

            let bytes = if len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
            };
            let req = match ppanel_sdk::InitRequest::decode(bytes) {
                Ok(r) => r,
                Err(e) => {
                    abi::deallocate(ptr, len);
                    return abi::encode_response(&ppanel_sdk::BoolResult {
                        success: false,
                        error: alloc::format!("decode init request: {}", e),
                    });
                }
            };

            abi::deallocate(ptr, len);

            abi::encode_response(&match #call {
                Ok(_) => ppanel_sdk::BoolResult { success: true, error: String::new() },
                Err(e) => ppanel_sdk::BoolResult { success: false, error: e },
            })
        }
    })
    .into()
}

// =========================================================================
// #[handler] — wraps a fn(HandleRequest) -> HandleResponse
// =========================================================================

#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let wasm_name = quote::format_ident!("__ppanel_handler_{}_wasm", fn_name);
    let export_str = match parse_export_attr(attr, format!("handle_{}", fn_name), "handler") {
        Ok(export) => export,
        Err(err) => return err,
    };
    let is_async = is_async(&input);

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name(req)) {
                Ok(resp) => resp,
                Err(block_err) => ppanel_sdk::HandleResponse {
                    status: 500,
                    body: alloc::format!("{{\"error\":\"block_on: {}\"}}", block_err).into_bytes(),
                    headers: Default::default(),
                },
            }
        }
    } else {
        quote! { #fn_name(req) }
    };

    (quote! {
        #input

        #[export_name = #export_str]
        pub extern "C" fn #wasm_name(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi;
            use ppanel_sdk::prost::Message as _;

            let bytes = if len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
            };
            let req = match ppanel_sdk::HandleRequest::decode(bytes) {
                Ok(r) => r,
                Err(e) => {
                    abi::deallocate(ptr, len);
                    return abi::encode_response(&ppanel_sdk::HandleResponse {
                        status: 500,
                        body: alloc::format!("{{\"error\":\"decode request: {}\"}}", e).into_bytes(),
                        headers: Default::default(),
                    });
                }
            };

            abi::deallocate(ptr, len);

            abi::encode_response(&#call)
        }
    })
    .into()
}

// =========================================================================
// #[start] — wraps a fn() -> Result<(), String>
// =========================================================================

#[proc_macro_attribute]
pub fn start(attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Err(err) = ensure_no_attr(attr, "start") {
        return err;
    }
    let input = parse_macro_input!(item as ItemFn);
    let is_async = is_async(&input);
    let fn_name = &input.sig.ident;

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name()) {
                Ok(r) => r,
                Err(e) => Err(alloc::format!("block_on: {}", e)),
            }
        }
    } else {
        quote! { #fn_name() }
    };

    (quote! {
        #input

        /// Exported as "start" — called by the host after init.
        /// The host uses `() -> ()` calling convention, so errors are
        /// logged rather than returned as protobuf.
        #[export_name = "start"]
        pub extern "C" fn __ppanel_start_wasm() {
            if let Err(e) = #call {
                ppanel_sdk::host::log::error(
                    &alloc::format!("plugin start error: {}", e)
                );
            }
        }
    })
    .into()
}

// =========================================================================
// #[stop] — wraps a fn() -> Result<(), String>
// =========================================================================

#[proc_macro_attribute]
pub fn stop(attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Err(err) = ensure_no_attr(attr, "stop") {
        return err;
    }
    let input = parse_macro_input!(item as ItemFn);
    let is_async = is_async(&input);
    let fn_name = &input.sig.ident;

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name()) {
                Ok(r) => r,
                Err(e) => Err(alloc::format!("block_on: {}", e)),
            }
        }
    } else {
        quote! { #fn_name() }
    };

    (quote! {
        #input

        /// Exported as "stop" — called by the host before unloading.
        /// The host uses `() -> ()` calling convention, so errors are
        /// logged rather than returned as protobuf.
        #[export_name = "stop"]
        pub extern "C" fn __ppanel_stop_wasm() {
            if let Err(e) = #call {
                ppanel_sdk::host::log::error(
                    &alloc::format!("plugin stop error: {}", e)
                );
            }
        }
    })
    .into()
}

// =========================================================================
// #[event_handler] — wraps a fn(EmitEventRequest) -> BoolResult
// =========================================================================

#[proc_macro_attribute]
pub fn event_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let wasm_name = quote::format_ident!("__ppanel_event_{}_wasm", fn_name);
    // Export name: "on_<fn_name>" — the host calls on_<event> convention.
    let export_str = match parse_export_attr(attr, format!("on_{}", fn_name), "event_handler") {
        Ok(export) => export,
        Err(err) => return err,
    };
    let is_async = is_async(&input);

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name(req)) {
                Ok(resp) => resp,
                Err(block_err) => ppanel_sdk::BoolResult {
                    success: false,
                    error: alloc::format!("block_on: {}", block_err),
                },
            }
        }
    } else {
        quote! { #fn_name(req) }
    };

    (quote! {
        #input

        #[export_name = #export_str]
        pub extern "C" fn #wasm_name(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi;
            use ppanel_sdk::prost::Message as _;

            let bytes = if len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
            };
            let req = match ppanel_sdk::EmitEventRequest::decode(bytes) {
                Ok(r) => r,
                Err(e) => {
                    abi::deallocate(ptr, len);
                    return abi::encode_response(&ppanel_sdk::BoolResult {
                        success: false,
                        error: alloc::format!("decode event: {}", e),
                    });
                }
            };

            abi::deallocate(ptr, len);

            abi::encode_response(&#call)
        }
    })
    .into()
}

// =========================================================================
// #[middleware] — wraps a fn(HandleRequest) -> MiddlewareResponse
// =========================================================================

#[proc_macro_attribute]
pub fn middleware(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let wasm_name = quote::format_ident!("__ppanel_mw_{}_wasm", fn_name);
    let export_str = match parse_export_attr(attr, format!("mw_{}", fn_name), "middleware") {
        Ok(export) => export,
        Err(err) => return err,
    };
    let is_async = is_async(&input);

    let call = if is_async {
        quote! {
            match ppanel_sdk::runtime::block_on(#fn_name(req)) {
                Ok(resp) => resp,
                Err(block_err) => ppanel_sdk::MiddlewareResponse {
                    action: "abort".into(),
                    status: 500,
                    body: alloc::format!("{{\"error\":\"block_on: {}\"}}", block_err).into_bytes(),
                    headers: Default::default(),
                },
            }
        }
    } else {
        quote! { #fn_name(req) }
    };

    (quote! {
        #input

        #[export_name = #export_str]
        pub extern "C" fn #wasm_name(ptr: i32, len: i32) -> i64 {
            use ppanel_sdk::abi;
            use ppanel_sdk::prost::Message as _;

            let bytes = if len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) }
            };
            let req = match ppanel_sdk::HandleRequest::decode(bytes) {
                Ok(r) => r,
                Err(e) => {
                    abi::deallocate(ptr, len);
                    return abi::encode_response(&ppanel_sdk::MiddlewareResponse {
                        action: "abort".into(),
                        status: 500,
                        body: alloc::format!("{{\"error\":\"decode request: {}\"}}", e).into_bytes(),
                        headers: Default::default(),
                    });
                }
            };

            abi::deallocate(ptr, len);

            abi::encode_response(&#call)
        }
    })
    .into()
}
