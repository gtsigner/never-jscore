mod context;
#[cfg(feature = "python")]
mod convert;
mod ops;
mod runtime;
mod storage;
mod early_return;  // New: Custom error for early return
mod crypto_ops;
mod encoding_ops;
mod timer_ops;
mod timer_real_ops;  // New: Real async timers
mod worker_ops;
mod fs_ops;
mod fetch_ops;
mod performance_ops;
mod random_state;  // New: Seedable RNG state management
mod random_ops;     // New: Random seed control operations
pub mod ffi;          // C FFI interface

#[cfg(feature = "python")]
use pyo3::prelude::*;
use std::sync::Once;

use context::Context;

// V8 platform initialization - must happen exactly once
static INIT: Once = Once::new();

fn ensure_v8_initialized() {
    INIT.call_once(|| {
        // Initialize V8 platform (required before creating any isolates)
        deno_core::JsRuntime::init_platform(None, false);
    });
}

/// never_jscore Python 模块
///
/// 类似 py_mini_racer 的设计，需要先创建 Context 实例才能使用。
/// 每个 Context 是完全独立的 JavaScript 执行环境。
///
/// Example:
///     ```python
///     import never_jscore
///
///     # 创建独立的 JS 执行环境
///     ctx = never_jscore.Context()
///
///     # 执行代码并加入全局作用域
///     ctx.eval("function add(a, b) { return a + b; }")
///     ctx.eval("function multiply(a, b) { return a * b; }")
///
///     # 调用函数
///     result = ctx.call("add", [1, 2])
///     print(result)  # 3
///
///     # 多个独立环境
///     ctx1 = never_jscore.Context()
///     ctx2 = never_jscore.Context()
///     # ctx1 和 ctx2 完全隔离，互不影响
///     ```
#[cfg(feature = "python")]
#[pymodule]
fn never_jscore(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Initialize V8 platform when module is first imported
    ensure_v8_initialized();

    // 只导出 Context 类
    // 不提供模块级函数，确保用户必须实例化才能使用
    m.add_class::<Context>()?;
    Ok(())
}
