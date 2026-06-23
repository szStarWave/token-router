//! C ABI for Electron and other native hosts (`cdylib` / `staticlib`).

use std::ffi::{CStr, c_char};
use std::path::Path;

use crate::embedded;

pub const TOKEN_OK: i32 = 0;
pub const TOKEN_ERR_ALREADY_RUNNING: i32 = 1;
pub const TOKEN_ERR_NOT_RUNNING: i32 = 2;
pub const TOKEN_ERR_INVALID_ARG: i32 = 3;
pub const TOKEN_ERR_INTERNAL: i32 = 4;

fn write_cstr(out: *mut c_char, out_len: usize, message: &str) {
    if out.is_null() || out_len == 0 {
        return;
    }
    let bytes = message.as_bytes();
    let n = bytes.len().min(out_len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, n);
        *out.add(n) = 0;
    }
}

fn map_error(err: &anyhow::Error) -> i32 {
    let msg = err.to_string();
    if msg.contains("already running") {
        TOKEN_ERR_ALREADY_RUNNING
    } else if msg.contains("not running") {
        TOKEN_ERR_NOT_RUNNING
    } else {
        TOKEN_ERR_INTERNAL
    }
}

/// Library version string (static, do not free).
#[unsafe(no_mangle)]
pub extern "C" fn token_router_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Start the gateway in a background thread. `config_path` may be null for the default path.
#[unsafe(no_mangle)]
pub extern "C" fn token_router_start(
    config_path: *const c_char,
    error_out: *mut c_char,
    error_out_len: usize,
) -> i32 {
    let path = if config_path.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(config_path) }.to_str() {
            Ok(s) if s.is_empty() => None,
            Ok(s) => Some(Path::new(s)),
            Err(e) => {
                write_cstr(error_out, error_out_len, &format!("invalid config_path: {e}"));
                return TOKEN_ERR_INVALID_ARG;
            }
        }
    };

    match embedded::start(path) {
        Ok(_) => TOKEN_OK,
        Err(e) => {
            let code = map_error(&e);
            write_cstr(error_out, error_out_len, &e.to_string());
            code
        }
    }
}

/// Stop the in-process gateway.
#[unsafe(no_mangle)]
pub extern "C" fn token_router_stop(error_out: *mut c_char, error_out_len: usize) -> i32 {
    match embedded::stop() {
        Ok(()) => TOKEN_OK,
        Err(e) => {
            let code = map_error(&e);
            write_cstr(error_out, error_out_len, &e.to_string());
            code
        }
    }
}

/// Returns 1 when the embedded gateway is running, otherwise 0.
#[unsafe(no_mangle)]
pub extern "C" fn token_router_is_running() -> i32 {
    i32::from(embedded::is_running())
}

/// Write the gateway base URL (e.g. `http://127.0.0.1:8787`) into `url_out`.
/// Returns the number of bytes written excluding the NUL terminator, or a negative error code.
#[unsafe(no_mangle)]
pub extern "C" fn token_router_gateway_url(url_out: *mut c_char, url_out_len: usize) -> i32 {
    if url_out.is_null() || url_out_len == 0 {
        return -TOKEN_ERR_INVALID_ARG;
    }

    let Some(url) = embedded::gateway_url() else {
        write_cstr(url_out, url_out_len, "gateway is not running");
        return -TOKEN_ERR_NOT_RUNNING;
    };

    if url.len() >= url_out_len {
        write_cstr(
            url_out,
            url_out_len,
            &format!("url buffer too small (need {} bytes)", url.len() + 1),
        );
        return -TOKEN_ERR_INVALID_ARG;
    }

    write_cstr(url_out, url_out_len, &url);
    url.len() as i32
}
