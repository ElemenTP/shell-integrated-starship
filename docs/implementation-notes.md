# Implementation Notes

本文档记录在将 Starship 改造为 shell 进程内插件过程中的经验和踩坑记录。May not up-to-date.

## 1. Starship 内部改造

### 1.1 Clone 派生问题

`Context` 中的缓存数据结构最初未派生 `Clone`：

- `StarshipConfig` — 仅有 `Default`，需添加 `Clone`（`toml::Table` 已实现 Clone）
- `DirContents` — 仅有 `Debug`，需添加 `Clone`（内部为 `HashSet`，均可 Clone）
- `GitRepo` — 包含 `ThreadSafeRepository`，**未添加 Clone**，因为 git repo 发现操作本身廉价，不值得缓存

**教训**：并非所有数据都需要缓存。优先缓存真正昂贵的操作（git status、目录扫描），
廉价的 repo 发现操作每次渲染重新执行即可。

### 1.2 Session 缓存与 OnceLock 的协作

`Context` 原本使用 `OnceLock` 实现 per-render 缓存——首次访问时计算，之后复用：

```rust
// 原始实现
pub fn dir_contents(&self) -> Result<&DirContents, &std::io::Error> {
    self.dir_contents.get_or_init(|| { /* 计算 */ }).as_ref()
}
```

Session 路由策略：在 `get_or_init` 的闭包中先检查 Session 缓存，
命中则直接返回，未命中则计算并同时写入 Session 和 OnceLock：

```rust
// 改造后（特性门控）
self.dir_contents.get_or_init(|| {
    // 1. 尝试 Session 缓存
    if let Some(session) = crate::session::find_session(session_id) {
        if let Some(cached) = session.get_dir_contents(current_dir) {
            return cached;
        }
    }
    // 2. 计算
    let result = DirContents::from_path_with_timeout(/* ... */);
    // 3. 写入 Session 缓存（下次渲染复用）
    if let Some(session) = crate::session::find_session(session_id) {
        session.put_dir_contents(current_dir.clone(), &result);
    }
    result
}).as_ref()
```

这种方式无需修改 `dir_contents()` 的方法签名和所有调用点。

### 1.3 git_status 的全局静态变量替换

原始代码使用 `static REPO_STATUS: parking_lot::Mutex<...>` 作为进程级缓存，
仅在 `current_dir` 变化时失效。这是整个 starship 代码库中唯一的跨调用状态。

改造为 Session 路由后：

- `session_id != 0` → 查询 Session 缓存（路径 + index/HEAD/packed-refs mtime 检测）
- `session_id == 0` → 回退到原始 `static` 行为（兼容独立二进制）

### 1.4 Cargo feature 的组织

`in-process` 特性应该是纯加法的（不改变默认行为）：

- `#[cfg(feature = "in-process")]` 只在添加新功能时使用
- 不改变现有公开 API 的行为
- 默认 feature 保持不变（`default = ["battery", "notify"]`）

验证方法：

```bash
# 默认构建不应受影响
cargo build && cargo test

# in-process 构建新增功能
cargo build --features in-process && cargo test --features in-process
```

## 2. Rust FFI 层

### 2.1 Rust 2024 Edition 的 unsafe 属性

Rust 2024 edition 中 `#[no_mangle]` 被标记为 unsafe 属性，必须写作：

```rust
#[unsafe(no_mangle)]
pub extern "C" fn ssp_init() -> *mut c_char { ... }
```

### 2.2 panic 隔离与错误返回

所有 FFI 导出函数必须包裹在 `catch_unwind` 中，防止 Rust panic 展开到 C 调用栈（未定义行为）。

可失败的导出函数统一使用以下错误协议：

- 返回 `char *`；
- `NULL` 表示成功；
- 非 `NULL` 是库分配的、以 NUL 结尾的 UTF-8 错误字符串，调用方必须用 `ssp_free()` 释放。

错误不存储在全局错误槽，也不存储在 session 中——它直接是本次调用的返回值。因此不存在
“调用方还未来得及读取就被其他调用覆盖”的竞态，也不需要 TLS 析构器。

```rust
macro_rules! ffi_guard_error {
    ($expr:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $expr)) {
            Ok(result) => result,          // *mut c_char：NULL 成功，非 NULL 错误
            Err(panic) => panic_to_error(panic), // panic 也转成分配的 error string
        }
    };
}
```

fork guard 也遵循同一约定，在子进程中直接 `return error_string(...)`。

### 2.3 内存管理约定

| 返回值 / 参数                  | 分配者                      | 释放者                   |
| ------------------------------ | --------------------------- | ------------------------ |
| 可失败调用的返回值（错误串）   | Rust`CString::into_raw()` | 调用者必须`ssp_free()` |
| `ssp_render` → `out` | Rust`CString::into_raw()` | 调用者必须`ssp_free()` |
| 全局 session                    | 库内部`Mutex<Option<Session>>` | `ssp_shutdown()` |
| `ssp_version` 返回值           | 静态字符串字面量            | 不可释放                 |

`ssp_free()` 本身不可能失败，因此保持 `void` 返回；`ssp_version()` 返回静态字符串，
两者不参与 `char *` 错误协议。

错误获取方式就是读取函数返回值：

```c
char *err = ssp_render(&in, &prompt);
if (err) {
    fprintf(stderr, "%s\n", err);
    ssp_free(err);
}
```

### 2.4 Rayon 线程池与 session 生命周期

FFI 使用 `starship::session::Session::new()` 创建 scoped rayon pool，并放在全局
`Mutex<Option<Session>>` 中。`ssp_shutdown()` 会取走并 drop 该 session，
触发 `Session::drop` 的 pool 终止逻辑；随后再次 `ssp_init()` 会构建全新的
pool、`SessionState`、缓存和统计。

因此 destroy→create 是受支持路径，且不会复用旧 session 的任何状态。zsh 的
`zmodload -u` 与 pwsh 的模块卸载都通过显式 destroy/shutdown 保证在 dlclose 前停止线程。

## 3. zsh 模块

### 3.1 构建依赖：zsh 头文件

zsh 可加载模块需要 `zsh.mdh`（由 `mkmakemod.sh` 从 `zsh.mdd` 生成）
以及大量 `.epro` 原型文件。这些文件在 configure + make 过程中生成。

构建步骤：

```bash
# 1. 配置 zsh 源码树
cd zsh-5.9.2 && ./configure --disable-gdbm --disable-pcre

# 2. 生成 Makemod 和 mdh 文件
top_srcdir="$PWD" bash Src/mkmakemod.sh Src Makemod

# 3. 生成 epro 文件
cd Src && make -f Makemod zsh.mdh
```

### 3.2 未定义符号的延迟解析

zsh 模块编译时引用了大量 zsh 内部符号（`getsparam`、`setsparam`、`ztrdup` 等），
这些符号在模块的 `.so` 中不存在，而是由宿主 zsh 二进制提供。

链接器标志：

```makefile
-Wl,--allow-shlib-undefined  # 允许未定义符号
-Wl,-z,lazy                  # 延迟绑定（用到时才解析）
```

模块由 `zmodload` 通过 `dlopen(RTLD_LAZY|RTLD_GLOBAL)` 加载时，
所有未定义符号自动从宿主 zsh 进程解析。

### 3.3 zsh 入口点的 `/**/` 标记

zsh 模块的六个入口点（`setup_`、`features_`、`enables_`、`boot_`、`cleanup_`、`finish_`）
前面必须有 `/**/` 注释标记，这是 zsh 构建系统 `makepro.awk` 的识别信号。

### 3.4 符号可见性

本机（Arch Linux，zsh 5.9.2）使用短符号名（`SHORTBOOTNAMES=yes`）——导出 `boot_` 即可。
部分平台的 zsh 需要长名称（如 `boot_starship_native`）。

验证方法：

```bash
nm zsh_src/build/starship_native.so | grep -E "^[0-9a-f]+ T (boot_|setup_|finish_)"
```

### 3.5 zsh 参数读写

- 读字符串参数：`getsparam("STARSHIP_CMD_STATUS")`
- 读整数参数：`getiparam("STARSHIP_JOBS_COUNT")`
- 读数组参数：`getaparam("STARSHIP_PIPE_STATUS")` —— zsh 数组以 NULL 终止
- 写字符串参数：`setsparam(name, ztrdup(val))` —— `ztrdup` 必须用 zsh 分配器

**关键**：向 zsh 写入字符串必须使用 `ztrdup()` 分配内存（zsh 使用自己的内存分配器），
不能用 `strdup()`。否则 zsh 后续释放时可能崩溃。

## 4. PowerShell 模块

### 4.1 P/Invoke 方案选择

从 `DllImport` 升级到 `LibraryImport`（.NET 7+ 源码生成器）：

| 特性       | DllImport          | LibraryImport          |
| ---------- | ------------------ | ---------------------- |
| 调用方式   | 运行时 IL stub     | 编译时生成 C# 代码     |
| 性能       | 每次调用有封送开销 | 接近原生调用           |
| AOT 兼容   | 有限               | 完全兼容 (NativeAOT)   |
| 字符串封送 | 仅 UTF-16          | 原生支持 UTF-8         |
| 可见性     | 黑盒               | 可跳转源码查看生成代码 |

关键差异：

- `class` → `partial class`
- `extern` → `partial`（源码生成器负责实现）
- `CallingConvention.Cdecl` → Linux/macOS 默认就是 Cdecl

### 4.2 原生库加载路径

PowerShell 模块的 `RootModule` 是托管 DLL，Native 库需要与其放在同一目录：

```
StarshipNative/bin/Release/net8.0/
├── StarshipNative.dll      ← RootModule (托管)
└── libstarship_ffi.so      ← Native 库（需手动复制）
```

`AppContext.BaseDirectory` 在模块加载时指向此目录。

### 4.3 跨平台库名处理

当前 `LibraryImport("libstarship_ffi.so")` 硬编码了 Linux 名称。
生产代码应使用条件编译或自定义导入解析器：

```csharp
#if WINDOWS
    private const string LibName = "starship_ffi.dll";
#elif OSX
    private const string LibName = "libstarship_ffi.dylib";
#else
    private const string LibName = "libstarship_ffi.so";
#endif
```

或使用 `NativeLibrary.SetDllImportResolver` 动态选择。

## 5. 构建系统

项目使用 CMake 构建，配置时通过 `FetchContent` 自动处理依赖。

### 5.1 自动依赖管理

CMake 配置时自动处理两个关键依赖：

- **Starship 源码**：`STARSHIP_SOURCE=AUTO` 时通过 `FetchContent` 从 GitHub clone
- **zsh 源码**：`ZSH_SOURCE=AUTO` 时从 SourceForge 下载 zsh 5.9.2 并自动
  `./configure --disable-gdbm --disable-pcre` + `mkmakemod.sh` + `make -f Makemod zsh.mdh`

两者都可通过 CMake 变量切换为本地路径。

### 5.2 Multi-Config 支持

Ninja Multi-Config 和 Visual Studio 等生成器**不使用** `CMAKE_BUILD_TYPE`。
构建时通过 `--config` 选择：

```bash
# 正确
cmake -B build -G "Ninja Multi-Config"
cmake --build build --config Release
```

Cargo 和 dotnet 通过 generator expression 自动匹配：

```cmake
set(CARGO_RELEASE_FLAG "$<IF:$<CONFIG:Release>,--release,>")
COMMAND ${DOTNET} build -c "$<CONFIG>" "${PWSH_PROJECT}"
```

### 5.3 CMake 中内联脚本的转义坑

`add_custom_target(COMMAND ...)` 中的命令字符串会被 CMake 进行变量展开。
zsh 的 `${#var}` 会被 CMake 解析为非法变量名。
解决方案：将 shell 脚本提取为独立文件。

### 5.4 动态链接方案

zsh 模块从最初静态链接 `libstarship_ffi.a`（21MB）改为动态链接 `libstarship_ffi.so`（15KB）。
`$ORIGIN` rpath 使 `.so` 可在同目录被找到，与 pwsh 模块共享同一个 `libstarship_ffi.so`。

```cmake
target_link_libraries(${ZSH_MODULE_NAME} PRIVATE "${FFI_SHARED}" pthread dl m)
set_target_properties(${ZSH_MODULE_NAME} PROPERTIES
    INSTALL_RPATH "$ORIGIN"
    BUILD_RPATH "${RUST_SRC_DIR}/target/release"
)
```

### 5.5 跨平台链接差异

```cmake
if(APPLE)
    target_link_options(${ZSH_MODULE_NAME} PRIVATE -Wl,-undefined,dynamic_lookup)
else()
    target_link_options(${ZSH_MODULE_NAME} PRIVATE -Wl,--allow-shlib-undefined -Wl,-z,lazy)
endif()
```

Linux 使用 `--allow-shlib-undefined` 允许未定义 zsh 符号；
macOS 使用 `-undefined dynamic_lookup` 达到同等效果。

## 6. 测试策略

### 分层测试

| 层级                | 测试目标                                 | 工具                                 |
| ------------------- | ---------------------------------------- | ------------------------------------ |
| Rust 单元测试       | `session.rs` 缓存逻辑、`ffi.rs` 导出 | `cargo test`                       |
| Starship 回归       | 默认构建所有 1238 个测试通过             | `cargo test` (feature off)         |
| Starship in-process | feature 开启时编译通过且测试不变         | `cargo test --features in-process` |
| C FFI 冒烟测试      | 通过 dlopen 调用所有 ssp_* 函数          | `gcc` + `dlopen`                 |
| zsh 集成测试        | 加载模块、渲染 prompt、统计缓存命中      | `zsh -c` 脚本                      |
| pwsh 集成测试       | Add-Type 加载、创建 Session、渲染、销毁  | `pwsh -NoProfile -Command`         |

### 测试环境注意

- zsh 测试需要 `zsh-5.9.2/Src/` 已运行过 `make -f Makemod` 生成头文件
- pwsh 测试需要 `libstarship_ffi.so` 与 `StarshipNative.dll` 在同一目录
- C 冒烟测试使用 `dlopen` 动态加载，不依赖链接器路径

## 7. 平台移植注意事项

### Linux → macOS (zsh)

- macOS 的 zsh 可能使用长符号名（`boot_zsh_native` 而非 `boot_`）
  需在 `module.c` 中添加条件编译或在构建配置中提供长名称变体
- 动态库后缀为 `.dylib` 而非 `.so`，`dlopen` 路径需调整

### Linux → Windows (pwsh)

- Native 库需要 Windows MSVC 工具链编译（`x86_64-pc-windows-msvc`）
- `starship` 的 `battery` 和 `notify` feature 在 Windows 交叉编译时可能有兼容性问题，
  已通过 `default-features = false` 规避
- PowerShell 的 `prompt` 函数机制与 zsh 不同：pwsh 在每次显示 prompt 时调用 `prompt` 函数，
  支持通过 `$function:prompt` 覆盖

## 8. 统计系统 (StatsStatus)

### 8.1 原子计数器

统计字段使用 `std::sync::atomic::AtomicU64` 而非 `Mutex<Stats>`：

```rust
pub struct StatsStatus {
    pub config_hits:          atomic::AtomicU64,
    pub config_misses:        atomic::AtomicU64,
    pub repo_status_hits:     atomic::AtomicU64,
    pub repo_status_misses:   atomic::AtomicU64,
    pub git_repo_hits:        atomic::AtomicU64,
    pub git_repo_misses:      atomic::AtomicU64,
    pub git_metrics_hits:     atomic::AtomicU64,
    pub git_metrics_misses:   atomic::AtomicU64,
    pub dir_contents_hits:    atomic::AtomicU64,
    pub dir_contents_misses:  atomic::AtomicU64,
    pub binary_path_hits:     atomic::AtomicU64,
    pub binary_path_misses:   atomic::AtomicU64,
    pub renders:              atomic::AtomicU64,
}
```

每个缓存操作直接 `fetch_add(1, Ordering::Relaxed)` 递增对应的原子计数器，
无需获取 Mutex 锁。对外通过 `Stats`（普通 struct）导出快照。

### 8.2 zsh 集成

`starship_stats` builtin 调用 `ssp_stats()` 获取快照，
设置 `STARSHIP_STATS_*` 整数参数并支持 `-v`（详细）和 `-q`（静默）选项：

```zsh
$ starship_stats -v
starship_native session stats (3 renders):
  total hits=2 misses=4 hit_rate=33.3%
  config:        hits=2 misses=1
  repo_status:   hits=0 misses=1
  git_repo:      hits=0 misses=1
  git_metrics:   hits=0 misses=0
  dir_contents:  hits=0 misses=1
  binary_path:   hits=0 misses=0

$ echo $STARSHIP_STATS_RENDERS
3
```

### 8.3 FFI 层

`ssp_stats_t` 结构体在 `ffi.h` 中定义，包含 13 个 `unsigned long long` 字段。
`ssp_stats()` 从 `StatsStatus` 中原子读取每个字段，组装为 C struct 返回。

## 9. 进程安全机制

嵌入 shell 进程的三个关键安全挑战及解决方案。

### 9.1 Fork Guard（防止 fork 子进程崩溃）

zsh 在以下场景会 `fork()` 且**不 exec**——子进程是父进程的字节级副本，
继承的 rayon/tokio 运行时处于损坏状态：

| 场景                       | fork 位置   | 子进程运行 builtin? |
| -------------------------- | ----------- | ------------------- |
| `$(starship_prompt)`     | exec.c:4724 | ✅ 危险             |
| `starship_prompt &`      | exec.c:2962 | ✅ 危险             |
| `starship_prompt \| cat`  | exec.c:3624 | ✅ 危险             |
| `(starship_prompt)`      | exec.c:3625 | ✅ 危险             |
| `starship_prompt > file` | —          | ❌ 不 fork          |
| `cat \| starship_prompt`  | —          | ❌ 不 fork（末位）  |

**解决方案**：全局 session 额外记录创建进程 PID（放在 `AtomicU32` 中，避免 fork
子进程先触碰可能被继承的 Mutex）：

```rust
static SESSION: Mutex<Option<starship::session::Session>> = Mutex::new(None);
static SESSION_PID: AtomicU32 = AtomicU32::new(0);

macro_rules! guard_fork {
    () => {
        let recorded_pid = SESSION_PID.load(Ordering::Relaxed);
        if recorded_pid != 0 && recorded_pid != std::process::id() {
            return error_string("refusing call in forked child process");
        }
    };
}
```

fork 子进程中的 FFI 调用返回非 `NULL` 错误串（`out` 保持为空），不触碰全局
Mutex、session 或 rayon。

pwsh/.NET 使用 `fork()+execve()`（立即 exec，中间无 managed 代码运行），**无需 guard**。

### 9.2 线程池清理（防止 dlclose 后 SIGSEGV）

**rayon 全局池无法关闭**——`build_global()` 创建的线程在进程生命周期内
永远存在，`dlclose` 后它们返回时访问已卸载代码 → SIGSEGV。

**解决方案**：使用 scoped `rayon::ThreadPool`，存储在 `Session` 中，
渲染时用 `pool.install()` 包裹：

```rust
pub struct Session {
    rayon_pool: parking_lot::Mutex<Option<rayon::ThreadPool>>,
    state: Arc<SessionState>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(pool) = self.rayon_pool.lock().take() {
            drop(pool); // 发送 terminate 信号（rayon Drop 不阻塞！）
            std::thread::sleep(Duration::from_millis(100)); // 等待 worker 退出
        }
    }
}
```

`ssp_shutdown()` 取走全局 `Option<Session>` 并 drop，因此 destroy→create
时旧 pool 已终止，新 create 会构建新 pool。

zsh 卸载链：`zmodload -u` → `cleanup_()` → `ssp_shutdown()` → `dlclose` 安全。

### 9.3 错误记录：直接随返回值返回

最初用 `thread_local!` 记录 last error。风险：TLS 首次在宿主线程
（zsh/pwsh 主线程）访问时注册析构器，`dlclose` 后析构器指针悬挂。
glibc 跳过已卸载 DSO 的析构器，但 **macOS 和 Windows 没有保护**。

之后考虑过“全局错误槽”和“每个 session 一个错误槽”，但两者都引入了调用方
“稍后再查询”的窗口；在窗口内其他调用可能覆盖槽内容。

**最终方案**：彻底取消错误槽。可失败函数直接返回 `char *`：

- `NULL`：成功；
- 非 `NULL`：本次调用的错误字符串，调用方用 `ssp_free()` 释放。

```rust
macro_rules! ffi_guard_error {
    ($expr:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $expr)) {
            Ok(result) => result,
            Err(panic) => panic_to_error(panic),
        }
    };
}
```

这样错误就是本次调用的返回值，不需要错误槽，不需要 TLS，也不会在 `dlclose` 后留下
悬挂析构器；不同调用之间更没有可互相覆盖的共享状态。`ssp_free()` 是唯一
不返回错误串的导出函数（它本身不可能失败），`ssp_version()` 返回静态字符串。

### 9.4 全局单 session 与 starship 静态变量审计

改为全局单 session 后，destroy→create 会创建全新的 `SessionState` 和 scoped rayon pool，
但 starship 源码中存在若干进程级静态状态。审计结论如下：

| 位置 | 状态 | 对 destroy→create 的影响 | 结论 |
| --- | --- | --- | --- |
| `context/mod.rs` `get_shell()` 内 `static SHELL: OnceLock<Shell>` | 缓存 `STARSHIP_SHELL` | 首次 render 后不再读取环境变量 | 单进程单 shell 模型下无问题；若同一进程内 destroy→create 后要切换 shell 类型，必须卸载/重载 dylib。记录为已知限制 |
| `modules/git_status.rs` `static REPO_STATUS` | 默认路径的全局 repo status 缓存 | `in-process` 且 `context.session = Some` 时完全绕过 | FFI 始终传 session，因此该静态不会跨 session 泄漏 |
| `print.rs` `static ANSI_REGEX: OnceLock<Regex>` | 固定正则缓存 | 无 session 数据 | 无影响 |
| `logger.rs` 的全局 logger | 日志文件句柄 | FFI 库不调用 `logger::init()` | 无影响 |
| `context/mod.rs` 的 `dir_contents` / `git_repo` `OnceLock` | 每次 `Context` 新建 | 无影响 | 缓存跨 render 由 `SessionState` 承载 |
| `modules/gcloud.rs`、`modules/rust.rs`、`modules/aws.rs` 中的 `OnceLock`/`OnceCell` | 每次 render 局部结构 | 无影响 | 不跨 session |
| `formatter/version.rs`、各 module 的 `LazyLock` | 函数内局部或常量 | 无影响 | 无状态或每次重建 |

因此：
- destroy→create 会得到空缓存和归零的统计，功能正确。
- 唯一需要记录的边界是 `STARSHIP_SHELL` 的进程级缓存；目标环境（一个 shell 进程只服务一种 shell）不受影响。
- 不需要为本次改造修改 `starship` 源码。

## 10. Fork/Unload 回归测试

`tests/test_fork_zsh.sh` 和 `tests/test_unload_zsh.sh` 覆盖：

| 测试                         | 验证内容                               |
| ---------------------------- | -------------------------------------- |
| `$()` 命令替换             | fork guard 拒绝，无崩溃，输出为空      |
| `&` 后台任务               | 同上                                   |
| 管道非末位                   | 同上                                   |
| `( )` 子 shell             | 同上                                   |
| `<( )` 进程替换            | 同上                                   |
| fork 后父进程                | 仍正常渲染                             |
| 5 次 load/render/unload 循环 | 无线程泄漏（`/proc/self/task` 计数） |

**测试注意事项**：被 guard 拒绝的 builtin 返回非零退出码，
`set -e` 下需要用 `if/else` 包装预期失败的分支。
