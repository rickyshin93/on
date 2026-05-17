# Project Learnings

## [Tooling] clippy `needless_raw_string_hashes` 会触发现有测试代码
- **日期**: 2026-05-13
- **错误假设**: 现有 `cargo clippy -- -D warnings` 在我的改动外不会有失败。
- **正确做法**: 新版 clippy（≥1.94）启用了 `needless_raw_string_hashes` lint，`src/config.rs` 中多处 `r#"..."#` 测试 YAML 字符串若内部不含 `"`，需要去掉 `#`。但若字符串里包含双引号（如 `echo "starting"`）则必须保留 `r#"..."#` 形式。
- **适用场景**: 升级 Rust 工具链或 CI 环境后 clippy 报新错时，先确认是否预存在；批量 sed 替换 raw string 时要看内容是否有 `"`。

## [TDD 纪律] 不要在 GREEN 阶段提前实现多余分支
- **日期**: 2026-05-13
- **错误假设**: 写 `from_flags` 时一次性把所有 only/short flag 合并逻辑都写完。
- **正确做法**: 严格按 RED→GREEN 一次只写最小代码通过当前测试；用后续 RED 驱动新分支。否则后写的测试虽然通过，但其实不能反映"测试先驱动出代码"的强度。
- **适用场景**: 使用 tdd skill 时，每个 cycle 先把现有实现回退到只够通过已有测试。

## [约束] 项目 forbid 了 unsafe_code，调系统调用必须用 nix
- **日期**: 2026-05-17
- **错误假设**: 想用 `libc::geteuid()` / `libc::kill(pid, 0)` 直接做系统调用。
- **正确做法**: `Cargo.toml [lints.rust] unsafe_code = "forbid"` 排除了 `libc::*` 直调。改用 `nix` crate 的安全封装：`nix::sys::signal::kill(Pid::from_raw(pid), None)` 返回 `Result<(), Errno>`，能区分 `EPERM`（存在无权限，应判活）vs `ESRCH`（不存在）。`nix::unistd::geteuid().is_root()` 替代手动比对 0。
- **适用场景**: 任何想直接调用 libc 的地方先看 nix 有没有现成包装。

## [生态] pnpm/npm 的孙子进程会脱离父进程组，只杀直接 child 不够
- **日期**: 2026-05-17
- **错误假设**: `pkill -P <shell_pid>` + 杀整个 PGID 就能清完 `pnpm dev` 的所有子孙进程。
- **正确做法**: pnpm 用 `cross-spawn` / `node` 启动子命令时，子命令会自己 `setsid()` / 脱离父 PG（rspack-node 是典型例子）。父进程被 SIGTERM 后，孙子进程留下来继续 listen 端口。要靠 `ps -axo pid,ppid` 拿全系统树 → BFS 收集 root 的所有后代 → 显式 kill 每个 PID。`pkill -P` 只走一层，不够深。
- **适用场景**: 任何用 npm/pnpm/yarn 包装 dev server 的 pane；validate 修复后 stop / restart 才能真正释放端口。

## [API] Command::arg 直接接受 &Path，不需要 to_str()
- **日期**: 2026-05-17
- **错误假设**: `Command::new("foo").arg(path.to_str().unwrap())` 是常见写法。
- **正确做法**: `arg` 接受 `AsRef<OsStr>`，而 `Path`/`PathBuf` 已实现，直接 `.arg(&path)` 即可。这能彻底干掉 `to_str().unwrap()`，让 clippy::unwrap_used 启用时不必特殊豁免。
- **适用场景**: 启用 `clippy::unwrap_used` 后清理 `.arg(...)` 调用。

## [TDD 策略] 拆 stdin 提示函数前，先抽出"纯解析"内核
- **日期**: 2026-05-17
- **错误假设**: 把 `io::stdin().read_line(...).unwrap()` 直接换成 `?` 就行。
- **正确做法**: 先抽 `parse_yes_default(input: &str) -> bool` / `parse_port_action(input: &str) -> PortAction` 这种纯函数，对它们写 RED 测试；再让 `prompt_*` 包装 IO + 调用纯函数。这样既能 TDD 驱动新行为（默认 Y、非交互短路），又把难测的 IO 边界压到最小。
- **适用场景**: 任何需要从 stdin 读取并解析用户响应的命令。

## [TDD 策略] 性能重构若改变可观察行为，依然需要 RED 测试
- **日期**: 2026-05-17
- **错误假设**: 把 fork `kill -0` 换成 `nix::kill(pid, 0)` 是纯性能优化，不需要新测试。
- **正确做法**: 新实现区分 `EPERM` 后，`is_pid_alive(1)`（非 root 调用 init）的返回值从 false → true，这是行为变化。要先写一个测它的 RED 测试驱动这次替换，否则现有测试集对"行为更正确了"无感知。技巧：用 `nix::unistd::geteuid().is_root()` 在测试里跳过 root 场景，CI 在 root 容器里也能过。
- **适用场景**: 任何看似"纯性能/纯重构"的改动，先问"哪一个外部可见行为会变？"，能答出就是该写 RED 测试的地方。
