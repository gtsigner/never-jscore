use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use crate::context::Context;
use crate::runtime::ensure_v8_initialized;

// Opaque pointer type
pub type ContextPtr = *mut Context;

#[no_mangle]
pub extern "C" fn never_jscore_init() {
    ensure_v8_initialized();
}

#[no_mangle]
pub extern "C" fn never_jscore_new(
    enable_extensions: c_int,
    enable_logging: c_int,
    random_seed: i64,
) -> ContextPtr {
    let ext = enable_extensions != 0;
    let log = enable_logging != 0;
    let seed = if random_seed < 0 { None } else { Some(random_seed as u32) };
    match Context::new(ext, log, seed) {
        Ok(ctx) => Box::into_raw(Box::new(ctx)),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_free(ptr: ContextPtr) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_eval(
    ptr: ContextPtr,
    code: *const c_char,
) -> *mut c_char {
    if ptr.is_null() || code.is_null() {
        return std::ptr::null_mut();
    }

    let ctx = unsafe { &*ptr };
    let c_str = unsafe { CStr::from_ptr(code) };
    let r_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    // execute_js returns a JSON string or an error
    match ctx.execute_js(r_str, true) {
        Ok(json) => {
            match CString::new(json) {
                Ok(c_string) => c_string.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        },
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

// Helper to execute script without return value (side effects only)
#[no_mangle]
pub extern "C" fn never_jscore_exec(
    ptr: ContextPtr,
    code: *const c_char,
) -> i32 {
    if ptr.is_null() || code.is_null() {
        return -1;
    }

    let ctx = unsafe { &*ptr };
    let c_str = unsafe { CStr::from_ptr(code) };
    let r_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match ctx.exec_script(r_str) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_compile(
    ptr: ContextPtr,
    code: *const c_char,
) -> i32 {
    // Compile is essentially exec_script (load into global scope)
    // Reuse exec_script logic
    never_jscore_exec(ptr, code)
}

#[no_mangle]
pub extern "C" fn never_jscore_gc(ptr: ContextPtr) {
    if !ptr.is_null() {
        let ctx = unsafe { &*ptr };
        let _ = ctx.request_gc();
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_get_stats(ptr: ContextPtr) -> usize {
    if !ptr.is_null() {
        let ctx = unsafe { &*ptr };
        return ctx.get_exec_count();
    }
    0
}

#[no_mangle]
pub extern "C" fn never_jscore_reset_stats(ptr: ContextPtr) {
    if !ptr.is_null() {
        let ctx = unsafe { &*ptr };
        ctx.reset_exec_count();
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_get_heap_statistics(ptr: ContextPtr) -> *mut c_char {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    let ctx = unsafe { &*ptr };
    match ctx.get_heap_stats() {
        Ok(stats) => {
            // Convert HashMap to JSON string manually or using serde_json
            // Since stats is HashMap<String, usize>, simple serde serialization works
            match serde_json::to_string(&stats) {
                Ok(json) => {
                    match CString::new(json) {
                        Ok(c_string) => c_string.into_raw(),
                        Err(_) => std::ptr::null_mut(),
                    }
                }
                Err(_) => std::ptr::null_mut(),
            }
        },
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn never_jscore_take_heap_snapshot(ptr: ContextPtr, file_path: *const c_char) -> i32 {
    if ptr.is_null() || file_path.is_null() {
        return -1;
    }
    let ctx = unsafe { &*ptr };
    let c_path = unsafe { CStr::from_ptr(file_path) };
    let path_str = match c_path.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    match ctx.take_heap_snapshot(path_str) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
