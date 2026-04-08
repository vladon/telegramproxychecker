//! Raw [tdjson](https://github.com/tdlib/td) C API. All `unsafe` for FFI stays in this module.

use std::ffi::{CStr, CString};
use std::time::Duration;

extern "C" {
    fn td_create_client_id() -> libc::c_int;
    fn td_send(client_id: libc::c_int, request: *const libc::c_char);
    fn td_receive(timeout: libc::c_double) -> *const libc::c_char;
    fn td_set_log_message_callback(
        max_verbosity_level: libc::c_int,
        callback: Option<extern "C" fn(libc::c_int, *const libc::c_char)>,
    );
}

pub(crate) fn create_client_id() -> i32 {
    unsafe { td_create_client_id() }
}

/// Sends a JSON request. TDLib copies the buffer before `td_send` returns.
pub(crate) fn send_json(client_id: i32, json: &str) -> Result<(), std::ffi::NulError> {
    let c = CString::new(json.as_bytes())?;
    unsafe {
        td_send(client_id, c.as_ptr());
    }
    Ok(())
}

/// Returns one JSON line or `None` on timeout. The C string from `td_receive` is copied before return.
pub(crate) fn receive_line(timeout: Duration) -> Option<String> {
    let secs = timeout.as_secs_f64().clamp(0.0, 86400.0);
    unsafe {
        let ptr = td_receive(secs);
        if ptr.is_null() {
            return None;
        }
        CStr::from_ptr(ptr).to_str().ok().map(String::from)
    }
}

pub(crate) fn set_log_callback(
    max_verbosity: i32,
    cb: Option<extern "C" fn(libc::c_int, *const libc::c_char)>,
) {
    unsafe {
        td_set_log_message_callback(max_verbosity, cb);
    }
}
