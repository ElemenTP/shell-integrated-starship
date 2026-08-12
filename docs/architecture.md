# Architecture: Starship as Native Shell Plugins

## 问题背景

Starship 作为 prompt 工具，每次渲染都需要启动独立进程 (`starship prompt ...`)，这带来两个问题：

1. **进程启动开销**：每次 prompt 渲染 fork + exec，Linux 下约 15-30ms，Windows 下更差
2. **状态无法保留**：每次调用都是全新进程，高开销操作（特别是 git 状态检测）反复执行

## 解决方案

将 Starship 编译为 C FFI 动态链接库，由 shell 进程直接加载，实现：

- **零进程创建**：prompt 渲染在 shell 进程内完成
- **状态缓存**：Session 对象在 shell 生命周期内持久化，缓存 7 类状态

## 整体架构

```
┌──────────────────────────────────────────────────────────────┐
│  rust_src/  (starship-ffi crate)                             │
│  cdylib + staticlib                                          │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  ffi.rs — extern "C" ssp_session_* API                 │  │
│  │  转换 C RenderInput → Properties，调用 print::get_prompt │  │
│  └───────────────────┬────────────────────────────────────┘  │
│                      │ path dependency, features = ["in-process"]
│  ┌───────────────────▼────────────────────────────────────┐  │
│  │  ../starship/                                          │  │
│  │  session.rs — Session / SessionState / 7 类缓存         │  │
│  │  context/mod.rs — 会话路由 (config, dir, git_repo)     │  │
│  │  modules/git_status.rs — REPO_STATUS 会话路由          │  │
│  │  modules/git_metrics.rs — diff 结果会话路由              │  │
│  └────────────────────────────────────────────────────────┘  │
└────────────────┬─────────────────────────┬───────────────────┘
                 │ cdylib(.so/.dylib/.dll) │
                 ▼                          ▼
┌─────────────────────────────┐  ┌──────────────────────────────┐
│  zsh_src/                   │  │  pwsh_src/                   │
│  module.c (C shim)          │  │  StarshipNative/ (.NET 8)    │
│  → starship_native zmodule  │  │  NativeMethods.cs (P/Invoke) │
│  链接libstarship_ffi cdylib │  │  Session.cs (托管封装)       │
│  内置命令:                  │  │  加载方式: Import-Module     │
│    starship_prompt           │  │  平台: Windows               │
│    starship_version          │  │                             │
|    starship_stats           |   |                            |
│  加载方式: zmodload         │  │                             │
│  平台: Linux / macOS        │  │                             │
└─────────────────────────────┘  └──────────────────────────────┘
```

## 核心设计决策

### 1. 特性门控修改 Starship

Starship 源码通过 `#[cfg(feature = "in-process")]` 特性门控进行最小化修改。
`rust_src` 通过 path dependency 引用。

**Starship 修改清单**：

| 文件                           | 修改                                                                                                                      |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| `Cargo.toml`                 | 添加`in-process = ["battery", "notify"]` feature                                                                        |
| `src/session.rs`             | **新建** — Session/SessionState/7 类缓存/统计                                                                      |
| `src/context/mod.rs`         | `Properties` 字段 pub；`session: Option<Arc<SessionState>>` 字段；会话路由 (config, dir_contents, git_repo, exec_cmd) |
| `src/modules/git_status.rs`  | `get_static_repo_status` 会话路由；`RepoStatus` 增加 `stashed_count`                                                |
| `src/modules/git_metrics.rs` | diff 结果会话缓存                                                                                                         |
| `src/config.rs`              | `StarshipConfig` 派生 `Clone`                                                                                         |
| `src/lib.rs`                 | `#[cfg(feature = "in-process")] pub mod session;`                                                                       |

### 2. 零模块修改策略

Starship 的 104 个模块实现均为纯函数 `fn(&Context) -> Option<Module>`，
无需逐个修改。状态持久化通过 **Context 的访问器方法路由到 Session 缓存** 实现。

### 3. 缓存体系

| # | 缓存项                          | 键                  | TTL    | 额外失效条件                                    |
| - | ------------------------------- | ------------------- | ------ | ----------------------------------------------- |
| 1 | Config + RootConfig             | (path, mtime, size) | —     | mtime 或 size 变化                              |
| 2 | Git repo status + stashed_count | repo_root           | 5000ms | `.git/index`, `HEAD`, `packed-refs` mtime |
| 3 | Git repo discovery              | current_dir         | 5000ms | —                                              |
| 4 | Git metrics diff                | repo_root           | 5000ms | 同上（与 repo_status 共享失效文件）             |
| 5 | Dir contents                    | path                | 5000ms | —                                              |
| 6 | Binary path resolution          | binary_name         | 5000ms | —                                              |
| 7 | External command output         | (binary, args)      | —     | 待实现 (P2)                                     |

环境变量控制：

- `STARSHIP_NATIVE_TTL_MS` — 覆盖 TTL（毫秒），默认 5000
- `STARSHIP_NATIVE_NO_CACHE=1` — 禁用所有缓存

### 4. FFI API 设计

```c
// 生命周期
ssp_session_t *ssp_session_create(void);
void ssp_session_shutdown(ssp_session_t *s);  // 关闭线程池，阻塞等待 worker 退出
void ssp_session_destroy(ssp_session_t *s);

// 渲染
int  ssp_session_render(ssp_session_t *s, const ssp_render_input_t *in, char **out);
void ssp_free(char *ptr);

// 元数据
const char *ssp_version(void);
void        ssp_last_error(char **out);  // 写入错误消息，调用者需 ssp_free

// 统计
int  ssp_session_stats(const ssp_session_t *s, ssp_stats_t *out);
```

`ssp_session_render` 返回 Rust 分配的内存，调用者必须通过 `ssp_free` 释放。

**进程安全机制**：

| 机制                         | 目的                                                                            |
| ---------------------------- | ------------------------------------------------------------------------------- |
| Fork guard (`creator_pid`) | zsh 的`$()`、`&`、管道会 fork 不 exec 的子进程；FFI 检测 PID 变化并拒绝调用 |
| Scoped rayon pool            | 替代全局池，`shutdown()` 可终止所有 worker 线程，dlclose 安全                 |
| 全局 Mutex 错误记录          | 避免 TLS 析构器在宿主线程上悬挂（macOS/Windows 无 DSO 保护）                    |

### 5. Shell 集成契约

**zsh**：

- 复用 stock `starship.zsh` 的 precmd/preexec 钩子
- 三个 builtin：`starship_prompt`（渲染）、`starship_stats`（统计）、`starship_version`（版本）
- `starship_prompt` 读取 zsh params → 调用 FFI 3 次 → 写入 `STARSHIP_PROMPT` / `STARSHIP_RPROMPT` / `STARSHIP_PROMPT2`
- `starship_stats` 调用 `ssp_session_stats` → 写入 `STARSHIP_STATS_*` 参数 + 打印摘要
- `PROMPT='$STARSHIP_PROMPT'` 使用参数展开，无 subshell
- `zmodload -u` 时 `cleanup_()` 依次调用 `ssp_session_shutdown` + `ssp_session_destroy`

**pwsh**：

- C# 托管封装 `Session` 包装 native `ssp_session_t`
- `prompt` 函数提取 PowerShell 状态 → `Session.Render()` → 返回字符串
- 通过 `[LibraryImport]` (source-gen) 加载 native 库

## 目录结构

```
shell-integrated-starship/
├── rust_src/                   # FFI crate (cdylib)
│   ├── Cargo.toml
│   └── src/lib.rs, ffi.rs
├── zsh_src/                    # zsh 模块
│   ├── ffi.h                   # C 头文件
│   ├── module.c                # zsh shim (starship_prompt/stats/version)
│   ├── starship-native.plugin.zsh # zsh 插件入口
├── pwsh_src/                   # PowerShell 模块
│   ├── StarshipNative/
│   │   ├── NativeMethods.cs    # P/Invoke 声明 + 原生库解析器
│   │   ├── Session.cs          # 托管封装
│   │   └── Environment.cs      # 跨平台环境变量
│   ├── StarshipNative.psm1    # 模块主体（prompt 逻辑）
│   ├── starship-native.psd1   # 模块清单
├── tests/
│   ├── ffi_smoke.c             # C FFI 冒烟测试
│   ├── test_zsh.sh             # zsh 集成测试
│   ├── test_fork_zsh.sh        # fork 安全回归测试
│   ├── test_unload_zsh.sh      # 卸载/重载循环测试
│   └── test_pwsh.ps1           # pwsh 集成测试
├── CMakeLists.txt              # CMake 构建
└── docs/
    ├── architecture.md         # 架构设计文档
    └── implementation-notes.md # 实现经验与踩坑记录
```

## 构建系统

项目使用 CMake 构建，支持 Ninja Multi-Config、Visual Studio 等生成器。
配置时自动处理依赖：

1. **Starship 源码**：`STARSHIP_SOURCE=AUTO`（默认）从 GitHub clone `in-process` 分支
2. **zsh 源码**：`ZSH_SOURCE=AUTO`（默认）下载 zsh 5.9.2 并自动 `./configure` + 生成头文件

```bash
# 一键构建（自动下载依赖）
cmake -B build -S .
cmake --build build --config Release

# 使用本地源码
cmake -B build -S . \
  -DSTARSHIP_SOURCE=/path/to/starship-source \
  -DZSH_SOURCE=/path/to/zsh-source
cmake --build build --config Release

# 仅构建特定模块
cmake --build build --config Release --target starship_native   # zsh
cmake --build build --config Release --target starship_pwsh     # pwsh

# 安装
cmake --install build --config Release --prefix ~/.local
```

CMake 选项：

| 选项                  | 默认值    | 说明                                                |
| --------------------- | --------- | --------------------------------------------------- |
| `STARSHIP_SOURCE`   | `AUTO`  | Starship 源码：`AUTO` 从 GitHub clone，或本地路径 |
| `ZSH_SOURCE`        | `AUTO`  | zsh 源码：`AUTO` 下载 zsh 5.9.2，或已配置的源码树 |
| `ZSH_VERSION`       | `5.9.2` | 下载 zsh 的版本                                     |
| `BUILD_ZSH_MODULE`  | `ON`    | 构建 zsh 可加载模块                                 |
| `BUILD_PWSH_MODULE` | `ON`    | 构建 PowerShell 二进制模块                          |
| `BUILD_TESTS`       | `ON`    | 构建测试目标                                        |

测试目标：

```bash
cmake --build build --config Release --target test-rust   # Rust 测试
cmake --build build --config Release --target test-zsh    # zsh 集成测试
cmake --build build --config Release --target test-pwsh   # pwsh 集成测试
cmake --build build --config Release --target check       # CTest (C 冒烟测试)
```
