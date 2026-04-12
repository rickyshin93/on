# Launch CLI - 设计文档

## 概述

`launch` 是一个 macOS CLI 工具，用 Rust 编写，目标是一键恢复开发工作环境。通过 YAML 配置文件定义每个项目的终端 pane、编辑器、浏览器页面，运行 `launch <project>` 即可自动完成所有启动操作。

## 技术栈

### Rust Crates

- `clap` — CLI 参数解析
- `serde` + `serde_yaml` — YAML 配置读写
- `serde_json` — JSON state 文件读写
- `dialoguer` — 无参数时 fuzzy 项目选择
- `colored` — 终端彩色输出
- `std::process::Command` — 调用系统命令

### 系统命令（macOS 自带）

- `osascript` — AppleScript 控制 iTerm2
- `open` — 打开浏览器
- `git` — 检查 git 状态
- `lsof` / `kill` — 端口检测与进程管理

### 文件系统布局

```
~/.launch/
  myproject.yaml        # 项目配置文件
  sideproject.yaml
  state/
    myproject.json      # 运行时 PID 记录
```

## 项目结构

```
src/
  main.rs        — CLI 入口，clap 定义，路由到子命令
  config.rs      — YAML 配置的结构体定义与加载/保存
  iterm.rs       — iTerm2 AppleScript 控制（开 tab、分 pane、关 tab）
  editor.rs      — 编辑器启动逻辑
  browser.rs     — 浏览器打开逻辑
  port.rs        — 端口检测（lsof）与进程 kill
  git.rs         — Git 状态检查
  state.rs       — PID 状态文件的读写与进程追踪
  process.rs     — launch/stop 的编排逻辑（串联以上模块）
```

## 配置结构

### YAML 配置示例

```yaml
name: myproject
iterm:
  layout: grid  # 可选，默认 vertical
  panes:
    - name: frontend
      dir: ~/projects/myproject/frontend
      cmd: npm run dev
    - name: backend
      dir: ~/projects/myproject/backend
      cmd: watchexec -e py python main.py
    - name: git
      dir: ~/projects/myproject
      # cmd 可选，不填则只 cd 到目录
editor:
  cmd: cursor   # 可选，默认 "code"
  folders:
    - ~/projects/myproject/frontend
    - ~/projects/myproject/backend
browser:        # 可选，整个 section 可省略
  - http://localhost:3000
  - https://github.com/me/myproject
```

### Rust 结构体

```rust
struct Config {
    name: String,
    iterm: Option<ItermConfig>,
    editor: Option<EditorConfig>,
    browser: Option<Vec<String>>,
}

struct ItermConfig {
    layout: Option<String>,  // "vertical"(默认) | "grid"
    panes: Vec<PaneConfig>,
}

struct PaneConfig {
    name: String,
    dir: String,
    cmd: Option<String>,
}

struct EditorConfig {
    cmd: Option<String>,     // 默认 "code"
    folders: Option<Vec<String>>,
}
```

所有顶级 section（`iterm`、`editor`、`browser`）均为 `Option`，可以只配部分功能。

## CLI 命令

| 命令 | 说明 |
|------|------|
| `launch <project>` | 启动项目环境 |
| `launch stop <project>` | 停止项目所有服务 |
| `launch stop --all` | 停止所有项目 |
| `launch list` | 列出所有项目及运行状态 |
| `launch edit <project>` | 用 $EDITOR 打开配置文件 |
| `launch new <project>` | 创建配置模板 |
| `launch` | dialoguer fuzzy 选择项目 |

## 核心流程

### `launch <project>` 启动流程

1. **加载配置** — 读取 `~/.launch/<project>.yaml`
2. **Git 状态检查** — 遍历所有 pane 的 `dir`（去重），执行 `git status --porcelain`
   - 有未提交改动则黄色警告，一次性展示所有目录状态
   - 统一询问一次是否继续
3. **端口提取** — 从 `browser` URL 解析 `localhost:<port>`，从 `cmd` 正则匹配 `--port`/`-p` 等
4. **端口冲突检测** — 对每个端口执行 `lsof -i :<port> -t`
   - 冲突时显示进程信息，逐个询问：Kill / Skip / Abort
5. **打开 iTerm2** — AppleScript 创建新 tab，在 tab 内分割 pane
   - 每个 pane 设置标题 `[projectname] panename`
   - cd 到目录，执行 cmd（如有）
6. **记录 PID** — 通过 pid 文件获取进程 PID，写入 `~/.launch/state/<project>.json`
7. **打开编辑器** — `<editor.cmd> <folders...>`
8. **打开浏览器** — `open <url>` 逐个打开

### `launch stop <project>` 停止流程

1. 读取 `~/.launch/state/<project>.json`
2. Kill 所有记录的 PID（先 SIGTERM，等待数秒，未退出则 SIGKILL）
3. AppleScript 关闭对应的 iTerm2 tab（按 tab 名前缀 `[projectname]` 匹配）
4. 删除 state 文件

## iTerm2 控制

### 布局策略

- **默认 vertical**（左右排列）— 每个 pane 保留完整高度，适合看日志
- **可选 grid**（2x2 网格）— 先左右分，再各自上下分

### AppleScript 实现

```applescript
tell application "iTerm2"
  tell current window
    create tab with default profile
    tell current session of current tab
      set name to "[myproject] frontend"
      write text "cd ~/projects/myproject/frontend && npm run dev"
    end tell
    tell current tab
      tell current session
        split vertically with default profile
      end tell
      tell last session
        set name to "[myproject] backend"
        write text "cd ~/projects/myproject/backend && ..."
      end tell
    end tell
  end tell
end tell
```

### Tab 关闭

遍历所有 tab 的 session，按名称前缀 `[myproject]` 匹配，关闭整个 tab。

### PID 获取

在 cmd 前包一层，将 PID 写入临时文件：

```bash
echo $$ > /tmp/.launch_<project>_<pane>.pid && exec <cmd>
```

启动后读取 pid 文件，写入 state JSON。

## 端口检测

### 端口提取规则

1. 从 `browser` URL 解析 — 匹配 `localhost:<port>` 或 `127.0.0.1:<port>`
2. 从 pane `cmd` 解析 — 正则匹配 `--port\s+\d+`、`-p\s+\d+`
3. 去重

### 冲突处理

```
lsof -i :<port> -t   →  获取占用进程 PID
ps -p <pid> -o comm= →  获取进程名
```

输出示例：
```
⚠ 端口 3000 被占用（进程: node, PID: 12345）
  [K]ill 进程并继续 / [S]kip 跳过 / [A]bort 退出？
```

## Git 状态检查

对每个 pane 的 `dir`（去重）执行 `git -C <dir> status --porcelain`。

输出示例：
```
⚠ ~/projects/myproject/frontend 有 3 个文件未提交
⚠ ~/projects/myproject/backend 有 1 个文件未提交
  继续启动？[Y/n]
```

## State 文件

```json
{
  "project": "myproject",
  "started_at": "2026-04-12T10:30:00",
  "panes": [
    { "name": "frontend", "pid": 12345 },
    { "name": "backend", "pid": 12346 }
  ]
}
```

## 其他命令

### `launch list`

```
项目          状态      Panes
myproject     运行中    frontend, backend, git
sideproject   已停止    -
```

读取所有 yaml 配置文件列出项目，结合 state 文件判断运行状态（检查 PID 是否存活）。

### `launch edit <project>`

执行 `$EDITOR ~/.launch/<project>.yaml`，`$EDITOR` 未设置时回退到 `vim`。

### `launch new <project>`

在 `~/.launch/<project>.yaml` 写入模板，然后自动打开编辑。

### `launch`（无参数）

用 `dialoguer::FuzzySelect` 列出所有项目名，选择后等同于 `launch <project>`。

## 错误处理

- 配置文件不存在 → 提示并建议 `launch new`
- iTerm2 未运行 → 提示用户先打开 iTerm2
- `~/.launch/` 目录不存在 → 首次运行时自动创建
- AppleScript 执行失败 → 打印 osascript 的 stderr
- 不做过度防御，信任系统命令的返回值

## 依赖清单

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1"
dialoguer = "0.11"
colored = "2"
```
