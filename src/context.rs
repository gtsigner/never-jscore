use anyhow::{Result, anyhow};
use deno_core::{JsRuntime, RuntimeOptions, error::JsError};
#[cfg(feature = "python")]
use pyo3::exceptions::PyException;
#[cfg(feature = "python")]
use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::types::{PyDict, PyList};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::rc::Rc;
use rand::SeedableRng;

#[cfg(feature = "python")]
use crate::convert::{json_to_python, python_to_json};
use crate::ops;
use crate::runtime::run_with_tokio;
use crate::storage::ResultStorage;

// ============================================
// 权限容器 - Web扩展需要
// ============================================

/// 权限容器，用于控制Web API的权限
///
/// 在JS逆向场景中，我们允许所有权限
struct PermissionsContainer;

// impl deno_web::TimersPermission for PermissionsContainer {
//     fn allow_hrtime(&mut self) -> bool {
//         true  // 允许高精度时间
//     }
// }

/// JavaScript 执行上下文
///
/// 每个 Context 包含一个独立的 V8 isolate 和 JavaScript 运行时环境。
/// 支持 Promise 和 async/await，默认自动等待 Promise 结果。
///
/// 使用方式类似 py_mini_racer：
/// ```python
/// ctx = never_jscore.Context(enable_extensions=True, enable_logging=False)
/// ctx.eval("function add(a, b) { return a + b; }")
/// result = ctx.call("add", [1, 2])
/// ```
#[cfg_attr(feature = "python", pyclass(unsendable))]
pub struct Context {
    runtime: RefCell<JsRuntime>,
    result_storage: Rc<ResultStorage>,
    exec_count: RefCell<usize>,
    extensions_loaded: bool,
    logging_enabled: bool,
    polyfill_loaded: RefCell<bool>,  // Track if polyfill has been loaded
    random_seed: Option<u32>,  // Store seed for deferred initialization
}

// JavaScript polyfill 代码
const JS_POLYFILL: &str = include_str!("dddd_js/js_polyfill.js");

/// 格式化 JavaScript 错误为人类可读的字符串
///
/// 将 deno_core 的 JsError 转换为清晰的错误消息，包含：
/// - 错误类型和消息
/// - 格式化的调用堆栈
/// - 源代码位置信息
fn format_js_error(error: &JsError) -> String {
    let mut output = String::new();

    // 1. 错误类型和消息
    if let Some(name) = &error.name {
        output.push_str(name);
        output.push_str(": ");
    }
    if let Some(message) = &error.message {
        output.push_str(message);
    }
    output.push('\n');

    // 2. 格式化的堆栈跟踪
    if let Some(stack) = &error.stack {
        // 清理堆栈信息，移除重复的错误消息
        let stack_lines: Vec<&str> = stack.lines().collect();

        // 跳过第一行（通常是重复的错误消息）
        for (i, line) in stack_lines.iter().enumerate() {
            if i == 0 && (line.contains(&error.name.as_deref().unwrap_or("")) ||
                         line.contains(&error.message.as_deref().unwrap_or(""))) {
                continue; // 跳过重复的错误消息
            }

            // 清理行内容
            let cleaned = line.trim();
            if !cleaned.is_empty() {
                output.push_str("  ");
                output.push_str(cleaned);
                output.push('\n');
            }
        }
    } else if !error.frames.is_empty() {
        // 如果没有 stack 字符串，从 frames 构建
        output.push_str("Stack trace:\n");
        for frame in &error.frames {
            output.push_str("  at ");

            if let Some(func_name) = &frame.function_name {
                output.push_str(func_name);
            } else {
                output.push_str("<anonymous>");
            }

            output.push_str(" (");

            if let Some(file_name) = &frame.file_name {
                output.push_str(file_name);
            } else if let Some(eval_origin) = &frame.eval_origin {
                output.push_str(eval_origin);
            } else {
                output.push_str("<eval>");
            }

            if let Some(line) = frame.line_number {
                output.push(':');
                output.push_str(&line.to_string());

                if let Some(col) = frame.column_number {
                    output.push(':');
                    output.push_str(&col.to_string());
                }
            }

            output.push_str(")\n");
        }
    }

    // 3. 源代码行（如果有）
    if let Some(source_line) = &error.source_line {
        output.push('\n');
        output.push_str("Source:\n  ");
        output.push_str(source_line);
        output.push('\n');
    }

    output
}

/// 从 anyhow::Error 中提取并格式化 JsError
///
/// 尝试从错误链中找到 JsError 并格式化，如果找不到则返回原始错误消息
fn format_error(error: anyhow::Error) -> String {
    // 尝试 downcast 到 JsError
    match error.downcast::<JsError>() {
        Ok(js_error) => format_js_error(&js_error),
        Err(original_error) => {
            // 不是 JsError，检查是否包含 JsError 的 cause chain
            let error_chain = format!("{:?}", original_error);

            // 尝试从调试输出中提取 JsError
            if error_chain.contains("JsError") {
                // 包含 JsError，但无法直接访问，尝试解析
                // 这是临时方案，返回简化的错误信息
                if let Some(msg_start) = error_chain.find("message: Some(\"") {
                    let msg_part = &error_chain[msg_start + 15..];
                    if let Some(msg_end) = msg_part.find("\")") {
                        let message = &msg_part[..msg_end];

                        if let Some(stack_start) = error_chain.find("stack: Some(\"") {
                            let stack_part = &error_chain[stack_start + 13..];
                            if let Some(stack_end) = stack_part.find("\"),") {
                                let stack = &stack_part[..stack_end];
                                // 清理转义字符
                                let cleaned_stack = stack.replace("\\n", "\n").replace("\\\"", "\"");
                                return format!("{}\n{}", message, cleaned_stack);
                            }
                        }

                        return message.to_string();
                    }
                }
            }

            // 无法提取 JsError，返回原始错误
            format!("{}", original_error)
        }
    }
}

impl Context {
    /// 创建新的 Context
    ///
    /// # Arguments
    /// * `enable_extensions` - 是否启用扩展（crypto, encoding 等）
    /// * `enable_logging` - 是否启用操作日志输出
    /// * `random_seed` - 随机数种子（可选）。如果提供，所有随机数 API 将使用固定种子
    pub fn new(enable_extensions: bool, enable_logging: bool, random_seed: Option<u32>) -> Result<Self> {
        let storage = Rc::new(ResultStorage::new());

        let mut extensions = vec![
            // Custom ops for result storage
            ops::pyexecjs_ext::init(storage.clone()),
        ];

        // 根据参数决定是否加载扩展
        if enable_extensions {
            extensions.push(crate::random_ops::random_ops::init());  // Random seed control (always loaded with extensions)
            extensions.push(crate::crypto_ops::crypto_ops::init());
            extensions.push(crate::encoding_ops::encoding_ops::init());
            // Real async timers (using channel + thread to avoid Tokio reactor issues)
            extensions.push(crate::timer_real_ops::timer_real_ops::init());
            extensions.push(crate::worker_ops::worker_ops::init());
            extensions.push(crate::fs_ops::fs_ops::init());
            extensions.push(crate::fetch_ops::fetch_ops::init());
            extensions.push(crate::performance_ops::performance_ops::init());

            // 新增: 浏览器环境 API
            extensions.push(crate::ops::web_storage::web_storage_ops::init());
            extensions.push(crate::ops::browser_env::browser_env_ops::init());
        }

        let runtime = JsRuntime::new(RuntimeOptions {
            extensions,
            ..Default::default()
        });

        // DON'T access OpState or Isolate during construction
        // Store the seed and set it on first execution instead

        // DON'T load polyfill here - defer to first execution to avoid isolate conflicts

        Ok(Context {
            runtime: RefCell::new(runtime),
            result_storage: storage,
            exec_count: RefCell::new(0),
            extensions_loaded: enable_extensions,
            logging_enabled: enable_logging,
            polyfill_loaded: RefCell::new(false),
            random_seed,
        })
    }

    /// Load polyfill on first execution
    fn ensure_polyfill_loaded(&self) -> Result<()> {
        if !self.extensions_loaded {
            return Ok(());
        }

        if *self.polyfill_loaded.borrow() {
            return Ok(());
        }

        // CRITICAL: Re-enter isolate before accessing runtime
        self.enter_isolate();

        let mut runtime = self.runtime.borrow_mut();

        // Set random seed if provided
        if let Some(seed) = self.random_seed {
            let op_state = runtime.op_state();
            let mut op_state = op_state.borrow_mut();
            // RngState is already initialized by the extension, just update it
            if let Some(rng_state) = op_state.try_borrow_mut::<crate::random_state::RngState>() {
                rng_state.seed = Some(seed as u64);
                rng_state.seeded_rng = Some(rand::rngs::StdRng::seed_from_u64(seed as u64));
            }
        }

        // Set logging flag
        let logging_flag = if self.logging_enabled { "true" } else { "false" };
        let logging_setup = format!("globalThis.__NEVER_JSCORE_LOGGING__ = {};", logging_flag);

        let _log_result = runtime
            .execute_script("<logging_setup>", logging_setup)
            .map_err(|e| anyhow!("Failed to setup logging: {:?}", e))?;

        let _result = runtime
            .execute_script("<polyfill>", JS_POLYFILL.to_string())
            .map_err(|e| anyhow!("Failed to load polyfill: {:?}", e))?;

        *self.polyfill_loaded.borrow_mut() = true;

        // Exit isolate after polyfill loading
        self.exit_isolate();

        Ok(())
    }

    /// 重新进入此 Context 的 Isolate
    ///
    /// 当存在多个 Context 实例时，V8 的 thread-local "current isolate" 可能指向错误的 isolate。
    /// 这个方法确保在执行任何 V8 操作前，正确的 isolate 是当前的。
    ///
    /// # Safety
    ///
    /// 这是一个 unsafe 操作，因为我们需要：
    /// 1. 从 RefCell 中获取原始指针
    /// 2. 调用 v8_isolate().enter() 来重新进入 isolate
    ///
    /// 但这是安全的，因为：
    /// - RefCell 确保了运行时的唯一性（通过 borrow_mut 检查）
    /// - 我们在同一个线程上操作
    /// - enter() 是可重入的（V8 文档保证）
    fn enter_isolate(&self) {
        unsafe {
            // SAFETY:
            // 1. runtime 被 RefCell 保护，as_ptr() 获取原始指针
            // 2. 我们立即解引用并调用 enter()，不存储指针
            // 3. enter() 本身是线程安全的（V8 保证）
            let runtime_ptr = self.runtime.as_ptr();
            let runtime = &mut *runtime_ptr;
            let isolate = runtime.v8_isolate();
            isolate.enter();
        }
    }

    /// 退出此 Context 的 Isolate
    ///
    /// 恢复之前的 isolate（如果有）。
    /// 应该在完成 V8 操作后调用。
    ///
    /// 重要：每个 enter_isolate() 都必须有对应的 exit_isolate()，
    /// 否则在 Context drop 时会导致 "Disposing the isolate that is entered" 错误。
    fn exit_isolate(&self) {
        unsafe {
            // SAFETY: 同 enter_isolate()
            let runtime_ptr = self.runtime.as_ptr();
            let runtime = &mut *runtime_ptr;
            let isolate = runtime.v8_isolate();
            isolate.exit();
        }
    }

    /// 执行脚本，将代码加入全局作用域（不返回值）
    ///
    /// 这个方法会直接执行代码并将定义的函数/变量加入全局作用域
    pub(crate) fn exec_script(&self, code: &str) -> Result<()> {
        // Ensure polyfill is loaded before first execution
        self.ensure_polyfill_loaded()?;

        // CRITICAL: Re-enter isolate to ensure it's current
        // This fixes the multi-Context issue where creating ctx2 breaks ctx1
        self.enter_isolate();

        let mut runtime = self.runtime.borrow_mut();

        // execute_script returns a v8::Global<v8::Value>
        // We let it drop immediately
        let _result = runtime
            .execute_script("<exec>", code.to_string())
            .map_err(|e| anyhow!("{}", format_error(e.into())))?;
        // v8::Global drops here

        // 简化的定时器处理：只运行 event loop 来处理微任务
        // 定时器通过 queueMicrotask 自动调度，依赖真实时间
        drop(runtime);

        // 使用 Tokio 运行 event loop (处理 queueMicrotask 队列)
        run_with_tokio(async {
            let mut rt = self.runtime.borrow_mut();

            // 运行 event loop 直到微任务队列为空
            rt.run_event_loop(Default::default()).await.ok();
        });

        // Exit isolate after operations complete
        self.exit_isolate();

        // 更新执行计数
        let mut count = self.exec_count.borrow_mut();
        *count += 1;

        // 每 100 次执行后提示 GC
        if *count % 100 == 0 {
            std::hint::black_box(());
        }

        Ok(())
    }

    /// 执行 JavaScript 代码并返回结果
    ///
    /// 根据 auto_await 参数决定是否自动等待 Promise。
    /// 注意：这个方法用于求值，代码在IIFE中执行，不会影响全局作用域
    ///
    /// Early Return 机制：
    /// - 当 JS 调用 __neverjscore_return__(value) 时，会抛出 EarlyReturnError
    /// - 该错误会携带返回值并中断 JS 执行
    /// - Rust 侧通过 downcast 检测并提取返回值
    pub(crate) fn execute_js(&self, code: &str, auto_await: bool) -> Result<String> {
        // Ensure polyfill is loaded before first execution
        self.ensure_polyfill_loaded()?;

        // CRITICAL: Re-enter isolate
        self.enter_isolate();

        self.result_storage.clear();

        if auto_await {
            // 异步模式：自动等待 Promise
            let result = run_with_tokio(async {
                let mut runtime = self.runtime.borrow_mut();

                // 序列化代码
                let code_json = serde_json::to_string(code)
                    .map_err(|e| anyhow!("Failed to serialize code: {}", e))?;

                // 简化的包装：只需要 async 函数和结果存储
                let wrapped_code = format!(
                    r#"
                    (async function() {{
                        const code = {};
                        const __result = await Promise.resolve(eval(code));

                        if (__result === undefined) {{
                            __getDeno().core.ops.op_store_result("null");
                            return null;
                        }}

                        try {{
                            const json = JSON.stringify(__result);
                            __getDeno().core.ops.op_store_result(json);
                            return __result;
                        }} catch(e) {{
                            const str = JSON.stringify(String(__result));
                            __getDeno().core.ops.op_store_result(str);
                            return __result;
                        }}
                    }})()
                    "#,
                    code_json
                );

                // 执行脚本
                let execute_result = runtime.execute_script("<eval_async>", wrapped_code);

                // 检查是否是 EarlyReturnError
                match execute_result {
                    Err(e) => {
                        // 检查是否是早期返回
                        if self.result_storage.is_early_return() {
                            // 提前返回：直接返回存储的值
                            let result = self.result_storage.take()
                                .ok_or_else(|| anyhow!("Early return but no result stored"))?;
                            let mut count = self.exec_count.borrow_mut();
                            *count += 1;
                            return Ok(result);
                        }
                        // 其他错误 - 格式化后返回
                        return Err(anyhow!("{}", format_error(e.into())));
                    }
                    Ok(result_handle) => {
                        // 正常执行，leak handle
                        std::mem::forget(result_handle);
                    }
                }

                // 运行 event loop 等待 Promise 完成
                let event_loop_result = runtime
                    .run_event_loop(Default::default())
                    .await;

                // 检查 event loop 是否遇到 EarlyReturnError
                if let Err(e) = event_loop_result {
                    // 检查是否是早期返回
                    if self.result_storage.is_early_return() {
                        // Event loop 中的提前返回
                        let result = self.result_storage.take()
                            .ok_or_else(|| anyhow!("Early return but no result stored"))?;
                        let mut count = self.exec_count.borrow_mut();
                        *count += 1;
                        return Ok(result);
                    }
                    // 其他错误 - 格式化后返回
                    return Err(anyhow!("{}", format_error(e.into())));
                }

                // 检查是否设置了 early return 标志（即使 event loop 正常完成）
                // 这处理了 eval() 内部调用 __neverjscore_return__ 的情况
                if self.result_storage.is_early_return() {
                    let result = self.result_storage.take()
                        .ok_or_else(|| anyhow!("Early return but no result stored"))?;
                    let mut count = self.exec_count.borrow_mut();
                    *count += 1;
                    return Ok(result);
                }

                // 正常完成：从 result_storage 获取结果
                let result = self
                    .result_storage
                    .take()
                    .ok_or_else(|| anyhow!("No result stored after event loop"))?;

                let mut count = self.exec_count.borrow_mut();
                *count += 1;

                Ok(result)
            });

            // Exit isolate after async operations complete
            self.exit_isolate();
            result
        } else {
            // 同步模式：不等待 Promise
            let mut runtime = self.runtime.borrow_mut();

            let code_json = serde_json::to_string(code)
                .map_err(|e| anyhow!("Failed to serialize code: {}", e))?;

            let wrapped_code = format!(
                r#"
                (function() {{
                    const code = {};
                    const __result = eval(code);
                    if (__result === undefined) {{
                        __getDeno().core.ops.op_store_result("null");
                        return null;
                    }}
                    try {{
                        const json = JSON.stringify(__result);
                        __getDeno().core.ops.op_store_result(json);
                        return __result;
                    }} catch(e) {{
                        const str = JSON.stringify(String(__result));
                        __getDeno().core.ops.op_store_result(str);
                        return __result;
                    }}
                }})()
                "#,
                code_json
            );

            let execute_result = runtime.execute_script("<eval_sync>", wrapped_code);

            // 检查是否是 EarlyReturnError
            match execute_result {
                Err(e) => {
                    // 检查是否是早期返回
                    if self.result_storage.is_early_return() {
                        // 提前返回
                        let result = self.result_storage.take()
                            .ok_or_else(|| anyhow!("Early return but no result stored"))?;
                        let mut count = self.exec_count.borrow_mut();
                        *count += 1;
                        return Ok(result);
                    }
                    return Err(anyhow!("{}", format_error(e.into())));
                }
                Ok(result_handle) => {
                    std::mem::forget(result_handle);
                }
            }

            // 从 storage 获取结果
            let result = self
                .result_storage
                .take()
                .ok_or_else(|| anyhow!("No result stored"))?;

            let mut count = self.exec_count.borrow_mut();
            *count += 1;

            // Exit isolate after sync operations complete
            self.exit_isolate();

            Ok(result)
        }
    }


    /// 请求垃圾回收
    pub fn request_gc(&self) -> Result<()> {
        self.enter_isolate();
        let mut runtime = self.runtime.borrow_mut();
        let _ =
            runtime.execute_script("<gc_hint>", "if (typeof gc === 'function') { gc(); } null;");
        drop(runtime);
        self.exit_isolate();
        Ok(())
    }

    /// 获取 V8 堆内存统计信息
    ///
    /// 返回当前 JavaScript 运行时的内存使用情况，包括总堆大小、已用大小等详细指标
    pub fn get_heap_stats(&self) -> Result<std::collections::HashMap<String, usize>> {
        self.enter_isolate();
        let mut runtime = self.runtime.borrow_mut();

        // 直接访问 V8 isolate 并获取堆统计信息
        let isolate = runtime.v8_isolate();
        let heap_stats = isolate.get_heap_statistics();

        let mut stats = std::collections::HashMap::new();
        stats.insert("total_heap_size".to_string(), heap_stats.total_heap_size());
        stats.insert("total_heap_size_executable".to_string(), heap_stats.total_heap_size_executable());
        stats.insert("total_physical_size".to_string(), heap_stats.total_physical_size());
        stats.insert("total_available_size".to_string(), heap_stats.total_available_size());
        stats.insert("used_heap_size".to_string(), heap_stats.used_heap_size());
        stats.insert("heap_size_limit".to_string(), heap_stats.heap_size_limit());
        stats.insert("malloced_memory".to_string(), heap_stats.malloced_memory());
        stats.insert("external_memory".to_string(), heap_stats.external_memory());
        stats.insert("peak_malloced_memory".to_string(), heap_stats.peak_malloced_memory());
        stats.insert("number_of_native_contexts".to_string(), heap_stats.number_of_native_contexts());
        stats.insert("number_of_detached_contexts".to_string(), heap_stats.number_of_detached_contexts());

        drop(runtime);
        self.exit_isolate();

        Ok(stats)
    }
    /// 获取执行统计信息
    pub fn get_exec_count(&self) -> usize {
        *self.exec_count.borrow()
    }

    /// 重置统计信息
    pub fn reset_exec_count(&self) {
        *self.exec_count.borrow_mut() = 0;
    }

    /// 导出 V8 堆快照到文件
    ///
    /// 导出完整的堆内存快照，可以用 Chrome DevTools 加载分析。
    pub fn take_heap_snapshot(&self, file_path: String) -> Result<()> {
        use std::io::Write;

        self.enter_isolate();
        let mut runtime = self.runtime.borrow_mut();
        let isolate = runtime.v8_isolate();

        // 创建输出文件
        let file = std::fs::File::create(&file_path)
            .map_err(|e| anyhow!("Cannot create file '{}': {}", file_path, e))?;

        let mut writer = std::io::BufWriter::new(file);

        // V8 会分多次调用回调函数，每次传递一块快照数据
        isolate.take_heap_snapshot(|chunk: &[u8]| {
            writer.write_all(chunk).is_ok()
        });

        // 确保所有数据写入磁盘
        writer.flush()
            .map_err(|e| anyhow!("Failed to write snapshot: {}", e))?;

        drop(runtime);
        self.exit_isolate();

        Ok(())
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // V8 runtime 会在 RefCell 销毁时自动清理
        // 注意：不要在这里调用 gc()，因为 Drop 可能在不同线程上被调用
        // 如果需要手动 GC，请在业务代码中显式调用 ctx.gc() 或使用 with 语句
    }
}

// ============================================
// Python Methods
// ============================================

#[cfg(feature = "python")]
#[pymethods]
impl Context {
    /// Python构造函数
    ///
    /// 创建一个新的JavaScript执行上下文
    ///
    /// Args:
    ///     enable_extensions: 是否启用扩展（crypto, encoding 等），默认 True
    ///                       - True: 启用所有扩展，自动注入 btoa/atob/md5/sha256 等函数
    ///                       - False: 纯净 V8 环境，只包含 ECMAScript 标准 API
    ///     enable_logging: 是否启用操作日志输出，默认 False
    ///                     - True: 输出所有扩展操作的日志（用于调试）
    ///                     - False: 不输出日志（推荐生产环境）
    ///     random_seed: 随机数种子（可选），用于确定性随机数生成
    ///                  - None: 使用系统随机数（非确定性）
    ///                  - int: 使用固定种子（确定性）
    ///                    所有随机数 API（Math.random、crypto.getRandomValues 等）
    ///                    将基于此种子生成，方便调试和算法对比
    ///
    /// Example:
    ///     ```python
    ///     import never_jscore
    ///
    ///     # 创建带扩展的上下文（默认）
    ///     ctx = never_jscore.Context()
    ///     result = ctx.evaluate("btoa('hello')")  # 可以直接使用 btoa
    ///
    ///     # 创建纯净 V8 环境
    ///     ctx_pure = never_jscore.Context(enable_extensions=False)
    ///     # 只有 ECMAScript 标准 API，无 btoa/atob 等
    ///
    ///     # 创建带日志的上下文（用于调试）
    ///     ctx_debug = never_jscore.Context(enable_logging=True)
    ///
    ///     # 创建带固定随机数种子的上下文（用于调试和算法对比）
    ///     ctx_seeded = never_jscore.Context(random_seed=12345)
    ///     r1 = ctx_seeded.evaluate("Math.random()")  # 确定性随机数
    ///     r2 = ctx_seeded.evaluate("Math.random()")  # 下一个确定性随机数
    ///
    ///     # 另一个相同种子的上下文将产生相同的随机数序列
    ///     ctx_seeded2 = never_jscore.Context(random_seed=12345)
    ///     r3 = ctx_seeded2.evaluate("Math.random()")  # r3 == r1
    ///     ```
    #[new]
    #[pyo3(signature = (enable_extensions=true, enable_logging=false, random_seed=None))]
    fn py_new(enable_extensions: bool, enable_logging: bool, random_seed: Option<u32>) -> PyResult<Self> {
        crate::runtime::ensure_v8_initialized();
        Self::new(enable_extensions, enable_logging, random_seed)
            .map_err(|e| PyException::new_err(format!("Failed to create context: {}", e)))
    }

    /// 编译JavaScript代码（便捷方法）
    ///
    /// 这是一个便捷方法，等价于 eval(code)。
    /// 执行代码并将函数/变量加入全局作用域。
    ///
    /// Args:
    ///     code: JavaScript 代码字符串
    ///
    /// Returns:
    ///     None
    ///
    /// Example:
    ///     ```python
    ///     ctx = never_jscore.Context()
    ///     ctx.compile('''
    ///         function add(a, b) { return a + b; }
    ///         function sub(a, b) { return a - b; }
    ///     ''')
    ///     result = ctx.call("add", [5, 3])
    ///     ```
    #[pyo3(signature = (code))]
    pub fn compile(&self, code: String) -> PyResult<()> {
        // 直接调用 exec_script，不经过 eval
        self.exec_script(&code)
            .map_err(|e| PyException::new_err(format!("Compile error: {}", e)))?;
        Ok(())
    }

    /// 调用 JavaScript 函数
    ///
    /// Args:
    ///     name: 函数名称
    ///     args: 参数列表
    ///     auto_await: 是否自动等待 Promise（默认 True）
    ///
    /// Returns:
    ///     函数返回值，自动转换为 Python 对象
    #[pyo3(signature = (name, args, auto_await=None))]
    pub fn call<'py>(
        &self,
        py: Python<'py>,
        name: String,
        args: &Bound<'_, PyAny>,
        auto_await: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let json_args = if args.is_instance_of::<PyList>() {
            let list = args.downcast::<PyList>()?;
            let mut vec_args = Vec::with_capacity(list.len());
            for item in list.iter() {
                vec_args.push(python_to_json(&item)?);
            }
            vec_args
        } else {
            vec![python_to_json(args)?]
        };

        let args_json: Vec<String> = json_args
            .iter()
            .map(|arg| serde_json::to_string(arg).unwrap())
            .collect();
        let args_str = args_json.join(", ");
        let call_code = format!("{}({})", name, args_str);

        let result_json = self
            .execute_js(&call_code, auto_await.unwrap_or(true))
            .map_err(|e| PyException::new_err(format!("Call error: {}", e)))?;

        let result: JsonValue = serde_json::from_str(&result_json)
            .map_err(|e| PyException::new_err(format!("JSON parse error: {}", e)))?;

        json_to_python(py, &result)
    }

    /// 执行代码并将其加入全局作用域
    ///
    /// 这个方法会执行JavaScript代码，并将定义的函数/变量保留在全局作用域中。
    /// 类似 py_mini_racer 的 eval() 方法。
    ///
    /// Args:
    ///     code: JavaScript 代码
    ///     return_value: 是否返回最后一个表达式的值（默认 False）
    ///     auto_await: 是否自动等待 Promise（默认 True）
    ///
    /// Returns:
    ///     如果 return_value=True，返回最后一个表达式的值；否则返回 None
    ///
    /// Example:
    ///     ```python
    ///     ctx = Context()
    ///     ctx.eval("function add(a, b) { return a + b; }")
    ///     result = ctx.call("add", [1, 2])  # 可以调用，因为add在全局作用域
    ///     ```
    #[pyo3(signature = (code, return_value=false, auto_await=None))]
    pub fn eval<'py>(
        &self,
        py: Python<'py>,
        code: String,
        return_value: bool,
        auto_await: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if return_value {
            // 需要返回值：使用包装的execute_js
            let result_json = self
                .execute_js(&code, auto_await.unwrap_or(true))
                .map_err(|e| PyException::new_err(format!("Eval error: {}", e)))?;

            let result: JsonValue = serde_json::from_str(&result_json)
                .map_err(|e| PyException::new_err(format!("JSON parse error: {}", e)))?;

            json_to_python(py, &result)
        } else {
            // 不需要返回值：直接执行脚本，加入全局作用域
            self.exec_script(&code)
                .map_err(|e| PyException::new_err(format!("Eval error: {}", e)))?;

            Ok(py.None().into_bound(py))
        }
    }

    /// 执行代码并返回结果（不影响全局作用域）
    ///
    /// 这个方法用于求值，代码在独立的作用域中执行，不会影响全局变量。
    ///
    /// Args:
    ///     code: JavaScript 代码
    ///     auto_await: 是否自动等待 Promise（默认 True）
    ///
    /// Returns:
    ///     表达式的值
    #[pyo3(signature = (code, auto_await=None))]
    pub fn evaluate<'py>(
        &self,
        py: Python<'py>,
        code: String,
        auto_await: Option<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let result_json = self
            .execute_js(&code, auto_await.unwrap_or(true))
            .map_err(|e| PyException::new_err(format!("Evaluate error: {}", e)))?;

        let result: JsonValue = serde_json::from_str(&result_json)
            .map_err(|e| PyException::new_err(format!("JSON parse error: {}", e)))?;

        json_to_python(py, &result)
    }

    /// 请求垃圾回收
    ///
    /// 注意：这只是向 V8 发送 GC 请求，V8 会根据自己的策略决定是否执行。
    fn gc(&self) -> PyResult<()> {
        self.request_gc()
            .map_err(|e| PyException::new_err(format!("GC error: {}", e)))
    }

    /// 获取执行统计信息
    ///
    /// Returns:
    ///     (exec_count,) 执行次数
    fn get_stats(&self) -> PyResult<(usize,)> {
        Ok((*self.exec_count.borrow(),))
    }

    /// 重置统计信息
    fn reset_stats(&self) -> PyResult<()> {
        *self.exec_count.borrow_mut() = 0;
        Ok(())
    }

    /// 获取 V8 堆内存统计信息
    ///
    /// 返回当前 JavaScript 运行时的详细内存使用情况。
    /// 所有大小值以字节为单位。
    ///
    /// Returns:
    ///     字典，包含以下键：
    ///     - total_heap_size: V8 分配的总堆大小（字节）
    ///     - total_heap_size_executable: 可执行堆的总大小（字节）
    ///     - total_physical_size: 实际占用的物理内存（字节）
    ///     - total_available_size: 可用堆大小（字节）
    ///     - used_heap_size: 当前已使用的堆大小（字节）
    ///     - heap_size_limit: 配置的堆大小限制（字节）
    ///     - malloced_memory: 通过 malloc 分配的内存（字节）
    ///     - external_memory: 外部对象使用的内存（字节）
    ///     - peak_malloced_memory: malloc 内存使用峰值（字节）
    ///     - number_of_native_contexts: 原生 V8 上下文数量
    ///     - number_of_detached_contexts: 已分离的上下文数量
    ///
    /// Example:
    ///     ```python
    ///     import never_jscore
    ///
    ///     ctx = never_jscore.Context()
    ///
    ///     # 执行一些 JS 代码
    ///     ctx.evaluate("let arr = new Array(1000000).fill(0)")
    ///
    ///     # 获取内存统计
    ///     stats = ctx.get_heap_statistics()
    ///     print(f"使用内存: {stats['used_heap_size'] / 1024 / 1024:.2f} MB")
    ///     print(f"总堆大小: {stats['total_heap_size'] / 1024 / 1024:.2f} MB")
    ///     print(f"堆使用率: {stats['used_heap_size'] / stats['total_heap_size'] * 100:.1f}%")
    ///     print(f"堆限制: {stats['heap_size_limit'] / 1024 / 1024:.2f} MB")
    ///
    ///     # 监控内存变化
    ///     before = ctx.get_heap_statistics()
    ///     ctx.evaluate("let big = new Array(10000000).fill(0)")
    ///     after = ctx.get_heap_statistics()
    ///     increase = after['used_heap_size'] - before['used_heap_size']
    ///     print(f"内存增加: {increase / 1024 / 1024:.2f} MB")
    ///     ```
    fn get_heap_statistics(&self, py: Python) -> PyResult<Py<PyDict>> {
        let stats = self.get_heap_stats()
            .map_err(|e| PyException::new_err(format!("Failed to get heap statistics: {}", e)))?;

        let dict = PyDict::new(py);
        for (key, value) in stats {
            dict.set_item(key, value)?;
        }

        Ok(dict.into())
    }

    /// 导出 V8 堆快照到文件
    ///
    /// 导出完整的堆内存快照，可以用 Chrome DevTools 加载分析。
    /// 这对于查找内存泄漏、分析对象引用关系、提取加密密钥等逆向工程任务非常有用。
    ///
    /// Args:
    ///     file_path: 快照文件保存路径（推荐使用 .heapsnapshot 扩展名）
    ///
    /// Example:
    ///     ```python
    ///     import never_jscore
    ///
    ///     ctx = never_jscore.Context()
    ///
    ///     # 执行包含敏感数据的 JS 代码
    ///     ctx.evaluate("""
    ///         let config = {
    ///             apiKey: 'secret_key_12345',
    ///             encryptionKey: 'aes_key_67890'
    ///         };
    ///         let data = encrypt(config);
    ///     """)
    ///
    ///     # 导出堆快照
    ///     ctx.take_heap_snapshot("memory.heapsnapshot")
    ///
    ///     # 使用 Chrome DevTools 分析：
    ///     # 1. 打开 Chrome -> F12 -> Memory 标签
    ///     # 2. 点击 "Load" 按钮
    ///     # 3. 选择 memory.heapsnapshot 文件
    ///     # 4. 在搜索框搜索 "secret_key" 或 "apiKey" 找到对象
    ///     # 5. 查看对象的引用链，了解数据流向
    ///     ```
    ///
    /// Tips:
    ///     - 快照文件是 JSON 格式，但可能很大（几十 MB）
    ///     - 可以对比两个快照找内存泄漏（before/after）
    ///     - 搜索已知字符串可以快速定位关键对象
    ///     - 查看对象的 Retainers 了解为什么对象没有被回收
    #[cfg(feature = "python")]
    #[pyo3(name = "take_heap_snapshot")]
    fn py_take_heap_snapshot(&self, file_path: String) -> PyResult<()> {
        self.take_heap_snapshot(file_path)
            .map_err(|e| PyException::new_err(format!("Snapshot error: {}", e)))
    }

    /// 上下文管理器支持：__enter__
    ///
    /// 允许使用 with 语句自动管理 Context 生命周期
    ///
    /// Example:
    ///     ```python
    ///     with never_jscore.Context() as ctx:
    ///         result = ctx.evaluate("1 + 2")
    ///         print(result)
    ///     # Context 自动清理
    ///     ```
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// 上下文管理器支持：__exit__
    ///
    /// 自动清理资源并请求 GC
    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        // 请求 GC，帮助释放资源
        self.gc()?;
        Ok(false)  // 不抑制异常
    }
}
