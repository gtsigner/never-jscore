# Linux 构建说明

## ✅ 当前方案：musllinux + 预编译 V8

**使用 musllinux_1_2 和预编译 V8** 以完全避免 glibc TLS 问题。

### 为什么选择 musllinux

musllinux 使用 musl libc 而不是 glibc，具有以下优势：
1. **完全静态链接**：避免所有动态链接和 TLS 问题
2. **无 TLS 模型冲突**：musl 不使用 `initial-exec` TLS 模型
3. **更小的二进制**：静态链接后体积更小
4. **更好的可移植性**：可在任何 Linux 发行版运行

### 已避免的问题

✅ 不再需要从源码编译 V8（避免 240+ 分钟构建时间）
✅ 不再需要 V8 142.1.0 源码构建（避免 `build/rust/known-target-triples.txt` 缺失）
✅ 完全避免 glibc TLS 链接错误：
```
error: relocation R_X86_64_TPOFF32 against v8::internal::g_current_isolate_
cannot be used with -shared
```

### 当前构建配置

```yaml
manylinux: musllinux_1_2
V8_FROM_SOURCE: 0
RUSTFLAGS: "-C target-feature=+crt-static -C link-arg=-static-libgcc"
```

## 如果 musllinux 构建失败的备选方案

### 方案 A：降级 deno_core 版本

编辑 `Cargo.toml`：
```toml
[dependencies]
deno_core = "0.320.0"  # 使用更早的稳定版本
```

这个版本使用的 V8 版本可能更稳定，且有更好的预编译支持。

### 方案 B：使用 musllinux 而不是 manylinux

编辑 `.github/workflows/build-wheels.yml`：
```yaml
manylinux: musllinux_1_2  # 使用 musl libc
```

musllinux 使用静态链接，完全避免 glibc TLS 问题。

### 方案 C：自定义 Docker 构建环境

在本地测试：
```bash
docker run -it --rm -v $(pwd):/workspace quay.io/pypa/manylinux2014_x86_64 bash
cd /workspace
export V8_FROM_SOURCE=0
maturin build --release --out dist
```

### 方案 D：使用 rusty_v8 预编译镜像

设置环境变量指向自定义 V8 构建：
```yaml
env:
  RUSTY_V8_MIRROR: https://github.com/denoland/rusty_v8/releases/download
  RUSTY_V8_ARCHIVE: librusty_v8_release_x86_64-unknown-linux-gnu.a
```

## TLS 问题的根本原因

预编译的 V8 使用 `initial-exec` TLS 模型，这在静态链接时没问题，但在作为 Python extension 被 `dlopen` 动态加载时会失败：

```
relocation R_X86_64_TPOFF32 cannot be used with -shared
```

解决方案：
1. **从源码编译**：使用 `V8_FROM_SOURCE=1` + `GN_ARGS` 指定 `global-dynamic` TLS 模型
2. **使用更宽容的环境**：manylinux2014/musllinux
3. **降级依赖**：使用没有此问题的旧版本

## 当前选择的权衡

- ✅ 构建速度快（~30-60 分钟 vs 240 分钟）
- ✅ 避免 V8 源码构建的复杂性
- ⚠️ 可能在某些 Linux 发行版上有兼容性问题
- ⚠️ manylinux2014 较旧，可能缺少新特性

如果需要最大兼容性，应考虑方案 B（musllinux）或回到从源码编译（需先解决 V8 142.1.0 构建问题）。
