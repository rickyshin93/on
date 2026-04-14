# AGENTS.md

## Project Overview

`launch` — A macOS CLI tool to restore your full dev environment with one command (iTerm2 panes, editor, browser).

- Language: Rust (edition 2021)
- Build: `cargo build`
- Test: `cargo test`
- Lint: `cargo clippy -- -D warnings`
- Format: `cargo fmt --check`

## Code Standards

- `unsafe` is forbidden (`unsafe_code = "forbid"`)
- Clippy pedantic warnings enabled, see `Cargo.toml [lints.clippy]`
- Formatting config in `rustfmt.toml`
- Ensure `cargo clippy` and `cargo fmt --check` pass before committing

## Project Structure

```
src/
  main.rs          — CLI entry point (clap)
  lib.rs           — Library entry point
  config.rs        — YAML config parsing (~/.launch/<project>.yaml)
  commands.rs      — Process tracking (PID)
  state.rs         — Runtime state management
  checks/
    mod.rs         — Checks module
    git.rs         — Git status checks
    port.rs        — Port conflict detection
  launcher/
    mod.rs         — Launcher module
    iterm.rs       — iTerm2 AppleScript integration
    editor.rs      — Editor launching
    browser.rs     — Browser opening
```

## Notes

- macOS + iTerm2 only
- Config path: `~/.launch/<project>.yaml`
- Keep README.md in sync when changing CLI arguments
