# Starship Native Shell Plugin

将 [Starship](https://starship.rs) prompt 编译为 Shell 原生动态库，实现进程内 prompt 渲染，
消除每条命令启动新进程的开销，并利用跨渲染缓存大幅减少重复的 git 状态检测等操作。

## 工作原理

```
传统模式（每命令 fork + exec）:
  $PROMPT → $(starship prompt --status=$? ...) → 进程启动 → git status → 输出 → 进程退出
  开销: ~15-30ms (Linux) / ~50+ms (Windows)

Native 模式（进程内渲染）:
  $PROMPT → $STARSHIP_PROMPT → starship_prompt builtin → FFI render → 写回参数
  开销: <1ms (缓存命中时)
```

## 支持平台

| Shell                | 平台                  | 加载方式                         |
| -------------------- | --------------------- | -------------------------------- |
| zsh                  | Linux, macOS          | `zmodload starship_native`     |
| pwsh (PowerShell 7+) | Windows, Linux, macOS | `Import-Module starship-native` |

## 编译

### 前置条件

- **CMake 3.21+**
- Rust 1.95+ (`cargo`)
- GCC 或 Clang (Linux/macOS, 用于编译 zsh shim)
- .NET SDK 8.0+ (仅 pwsh 模块)

### 一键构建（自动下载依赖）

CMake 会自动下载 starship 源码 (GitHub) 和 zsh 源码 (SourceForge)，无需手动准备：

```bash
cmake -B build -S .
cmake --build build --config Release
```

### 使用本地源码

```bash
cmake -B build -S . \
  -DSTARSHIP_SOURCE=/path/to/starship_source \
  -DZSH_SOURCE=/path/to/zsh_source
cmake --build build --config Release
```

### 仅构建特定模块

```bash
# 仅 zsh 模块
cmake -B build ... -DBUILD_PWSH_MODULE=OFF

# 仅 pwsh 模块
cmake -B build ... -DBUILD_ZSH_MODULE=OFF
```

### 安装

```bash
# zsh: 安装到 ~/.local/lib/zsh
cmake --install build --config Release --prefix ~/.local

# 系统级安装
sudo cmake --install build --config Release --prefix /usr/local
```

### CMake 选项

| 选项                  | 默认值    | 说明                                                    |
| --------------------- | --------- | ------------------------------------------------------- |
| `STARSHIP_SOURCE`   | `AUTO`  | Starship 源码：`AUTO` 从 GitHub clone，或本地路径     |
| `ZSH_SOURCE`        | `AUTO`  | zsh 源码：`AUTO` 下载 zsh 5.9.2，或已配置的源码树路径 |
| `ZSH_VERSION`       | `5.9.2` | 下载 zsh 的版本（ZSH_SOURCE=AUTO 时）                   |
| `BUILD_ZSH_MODULE`  | `ON`    | 构建 zsh 可加载模块                                     |
| `BUILD_PWSH_MODULE` | `ON`    | 构建 PowerShell 二进制模块                              |
| `BUILD_TESTS`       | `ON`    | 构建测试目标                                            |

生成器选择：

```bash
# Linux/macOS
-G "Ninja"

# Windows (Visual Studio)
-G "Visual Studio 17 2022"
```

## 使用

### zsh

本项目以标准 `*.plugin.zsh` 约定提供插件文件（仓库根目录
`starship-native.plugin.zsh`），因此可以被主流 zsh 插件管理器识别。

#### 方式一：插件管理器（推荐）

先编译并安装（安装前缀任意，插件会自动搜索）：

```bash
cmake -B build -S .
cmake --build build --config Release
cmake --install build --config Release --prefix ~/.local
```

然后在插件管理器里加载本仓库：

| 管理器    | 配置示例                                                                                                 |
| --------- | -------------------------------------------------------------------------------------------------------- |
| oh-my-zsh | `git clone <repo> ~/.oh-my-zsh/custom/plugins/starship-native`，然后 `plugins=(... starship-native)` |
| zinit     | `zinit light <user>/shell-integrated-starship`                                                         |
| antigen   | `antigen bundle <user>/shell-integrated-starship`                                                      |
| zplug     | `zplug "<user>/shell-integrated-starship", use:"starship-native.plugin.zsh"`                           |
| zgen      | `zgen load <user>/shell-integrated-starship`                                                           |
| sheldon   | `sheldon add starship-native --github <user>/shell-integrated-starship`                                |

插件加载时按以下顺序自动定位编译产物：

1. `$STARSHIP_NATIVE_DIR`（显式指定，最高优先级）
2. 插件脚本所在目录（即 `cmake --install` 布局 `lib/zsh/`）
3. 常见安装前缀（`~/.local/lib/zsh`、`/usr/local/lib/zsh`、`/opt/starship-native/lib/zsh` 等）

若编译产物不在上述位置，可在 `.zshrc` 中指定：

```zsh
export STARSHIP_NATIVE_DIR="$HOME/.local/lib/zsh"
```

#### 方式二：直接 source

在 `.zshrc` 中添加：

```zsh
# 加载插件
source ~/.local/lib/zsh/starship-native.plugin.zsh

# 可选：自定义 TTL（毫秒），默认 5000
export STARSHIP_NATIVE_TTL_MS=10000

# 可选：调试缓存
# export STARSHIP_NATIVE_NO_CACHE=1
```

`starship-native.plugin.zsh` 自动完成：

1. 通过 `zmodload` 加载 `starship_native动态库`
2. 注册 precmd/preexec 钩子（追踪命令状态、耗时等）
3. 在每次 prompt 前调用 `starship_prompt` builtin 渲染 prompt
4. 设置 `PROMPT='$STARSHIP_PROMPT'`、`RPROMPT='$STARSHIP_RPROMPT'`、`PROMPT2='$STARSHIP_PROMPT2'`

#### Builtin 命令

| 命令                 | 说明                                                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `starship_prompt`  | 渲染 prompt。读取 zsh 参数，调用 FFI 3 次 (Main/Right/Continuation)，写入`STARSHIP_PROMPT` / `STARSHIP_RPROMPT` / `STARSHIP_PROMPT2` |
| `starship_stats`   | 显示缓存统计。`-v` 详细模式，`-q` 静默模式（仅设置参数）                                                                               |
| `starship_version` | 显示原生库版本，`-q` 静默模式（仅设置参数）                                                                                              |

**fork 安全**：在 `$(...)`、`&`、管道非末位、子 shell 中调用 builtin 时，
fork guard 会拒绝调用并返回安全默认值，不会崩溃。见「进程安全」章节。

#### 缓存统计

`starship_stats` 会显示各缓存的命中率：

```
starship_native session stats (150 renders):
  total hits=892 misses=8 hit_rate=99.1%
  config:        hits=149 misses=1
  repo_status:   hits=147 misses=3
  git_repo:      hits=149 misses=1
  git_metrics:   hits=149 misses=1
  dir_contents:  hits=149 misses=1
  binary_path:   hits=149 misses=1
```

同时设置以下 zsh 参数：

| 参数                                               | 说明                 |
| -------------------------------------------------- | -------------------- |
| `STARSHIP_STATS_RENDERS`                         | 总渲染次数           |
| `STARSHIP_STATS_CONFIG_HITS` / `_MISSES`       | 配置缓存             |
| `STARSHIP_STATS_REPO_STATUS_HITS` / `_MISSES`  | Git 状态缓存         |
| `STARSHIP_STATS_GIT_REPO_HITS` / `_MISSES`     | Git 仓库发现缓存     |
| `STARSHIP_STATS_GIT_METRICS_HITS` / `_MISSES`  | Git 文件行数统计缓存 |
| `STARSHIP_STATS_DIR_CONTENTS_HITS` / `_MISSES` | 目录扫描缓存         |
| `STARSHIP_STATS_BINARY_PATH_HITS` / `_MISSES`  | 二进制路径缓存       |

### pwsh

模块为「脚本模块 + 二进制模块」混合结构：

- `starship-native.psm1` — 模块主体（prompt 逻辑、原生库定位、辅助命令）
- `starship-native.psd1` — 模块清单（`RootModule` 指向 `.psm1`）
- `StarshipNative.dll` — C# P/Invoke 封装（通过清单的 `NestedModules` 加载）

```powershell
# 方式一：从安装目录导入
Import-Module /path/to/prefix/share/pwsh/modules/starship-native

# 方式二：安装到 PSModulePath 后按名称导入（推荐）
$env:PSModulePath += ":$HOME/.local/share/powershell/Modules"
Import-Module starship-native

# prompt 函数会自动替换；卸载模块时自动恢复原 prompt

# pwsh 环境变量需用 StarshipEnvironment 设置（保证原生库可见）：
Set-StarshipEnv STARSHIP_NATIVE_TTL_MS 10000

# 辅助命令
Get-StarshipNativeVersion   # 原生库版本
Get-StarshipNativeStats     # 缓存统计
```

> 原生库（`libstarship_ffi.so` / `.dylib` / `starship_ffi.dll`）默认与
> `StarshipNative.dll` 同目录；如需换位置，设置 `$env:STARSHIP_FFI_PATH`。

### pwsh 环境变量

PowerShell 的 `$env:VAR = "value"` 在Unix系统上不会更新原生 OS 环境，
in-process 的 Rust 库无法看到。模块提供 `Set-StarshipEnv` / `Remove-StarshipEnv`
命令，同时写入两套环境：

```powershell
Set-StarshipEnv STARSHIP_NATIVE_TTL_MS 10000
Set-StarshipEnv STARSHIP_CONFIG /custom/path/starship.toml
Remove-StarshipEnv STARSHIP_NATIVE_NO_CACHE
```

## 环境变量

| 变量                         | 默认值   | 说明                             |
| ---------------------------- | -------- | -------------------------------- |
| `STARSHIP_SHELL`           | (自动)   | shell 类型：`zsh`、`pwsh`    |
| `STARSHIP_NATIVE_TTL_MS`   | `5000` | 缓存 TTL（毫秒）                 |
| `STARSHIP_NATIVE_NO_CACHE` | (未设置) | 设为`1` 禁用所有缓存（调试用） |

## 缓存体系

Session 在 shell 生命周期内持久化，以下 6 类状态自动缓存：

| 缓存                         | 内容                                                        | 失效策略                                           |
| ---------------------------- | ----------------------------------------------------------- | -------------------------------------------------- |
| **Config**             | `starship.toml` 解析结果 + `StarshipRootConfig`         | 文件 mtime/size 变化                               |
| **Git repo status**    | 完整 git 状态 (staged/modified/untracked/... + stash count) | TTL +`.git/index`/`HEAD`/`packed-refs` mtime |
| **Git repo discovery** | repo 对象 (branch/remote/fsmonitor)                         | TTL (key = current_dir)                            |
| **Git metrics**        | 文件增删行数 (blob diff)                                    | TTL +`.git/index`/`HEAD`/`packed-refs` mtime |
| **Dir contents**       | 目录文件列表 (扩展名/文件名)                                | TTL (key = path)                                   |
| **Binary path**        | `which::which` 结果 (二进制路径)                          | TTL (key = binary_name)                            |

## 运行测试

```bash
# Rust 单元测试
cd starship && cargo test && cargo test --features in-process
cd rust_src && cargo test --release

# C 冒烟测试
cmake --build build --config Release --target ffi_smoke
./tests/ffi_smoke rust_src/target/release/libstarship_ffi.so (or libstarship_ffi.dylib)

# zsh 集成测试
cmake --build build --config Release --target test-zsh

# fork 安全回归测试（$(...)、&、管道、子shell）
MODULE_DIR=zsh_src/build/Release zsh tests/test_fork_zsh.sh

# 卸载/重载循环测试（线程泄漏检查）
MODULE_DIR=zsh_src/build/Release zsh tests/test_unload_zsh.sh

# pwsh 集成测试
cmake --build build --config Release --target test-pwsh
```

## 进程安全

嵌入式库与 shell 进程共存，三个关键安全机制：

| 机制                    | 问题                                                                                   | 方案                                                                          |
| ----------------------- | -------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Fork guard**    | zsh 的`$()`、`&`、管道会 fork 不 exec 的子进程，继承损坏的 rayon 运行时 → SIGSEGV | Session 记录创建时 PID，FFI 入口检测并拒绝 fork 子进程调用                    |
| **线程池清理**    | rayon 全局池无法关闭；dlclose 后线程访问已卸载代码                                     | 改用 scoped`ThreadPool`，`ssp_session_shutdown()` 发信号+等待 worker 退出 |
| **无 TLS 析构器** | `thread_local!` 在宿主线程注册析构器，dlclose 后悬挂（macOS/Windows 无保护）         | 改用全局`Mutex`，无每线程状态                                               |

详细分析见 `docs/implementation-notes.md` §9。

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
│   ├── starship-native.psm1    # 模块主体（prompt 逻辑）
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

## License

ISC — 与 Starship 相同。
