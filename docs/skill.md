# on — Dev Environment Launcher

You are being asked to help a user manage their dev environment using the `on` CLI tool.

## What is `on`?

`on` is a CLI that launches a complete dev environment (terminal panes, editor, browser tabs) from a single YAML config file. Configs live in `~/.on/<project>.yaml`.

## Install

```bash
brew install rickyshin93/tap/on
```

Or from source: `cargo install --path .` (requires Rust toolchain).

## Commands

| Command | Description |
|---------|-------------|
| `on <project>` | Launch a project |
| `on` | Fuzzy-select from configured projects |
| `on init` | Auto-detect project structure and create config |
| `on new <project>` | Create a blank config from template |
| `on edit <project>` | Edit config in $EDITOR |
| `on clone <old> <new>` | Clone an existing config |
| `on restart <project>` | Stop + start a project |
| `on stop <project>` | Stop a project |
| `on stop --all` | Stop all running projects |
| `on status <project>` | Show detailed status (panes, ports, uptime) |
| `on log <project> [pane]` | View pane output logs |
| `on list` | List all projects and status |
| `on doctor` | Check environment health |

## Config Format

```yaml
name: myproject
extends: base                  # optional: inherit from ~/.on/base.yaml

terminal:
  type: iterm                  # iterm (macOS default) | tmux (Linux default)
  layout: vertical             # vertical | grid
  max_panes_per_tab: 4         # 2-8, splits across tabs
  panes:
    - name: backend
      dir: ~/projects/myproject/api
      cmd: cargo run
      env:                     # per-pane environment variables
        RUST_LOG: debug
        DATABASE_URL: postgres://localhost/mydb
    - name: frontend
      dir: ~/projects/myproject/web
      cmd: pnpm dev
    - name: shell
      dir: ~/projects/myproject

editor:
  cmd: cursor                  # code | cursor | vim | ...
  folders:
    - ~/projects/myproject/api
    - ~/projects/myproject/web
  # workspace: ~/.on/myproject.code-workspace

browser:
  - http://localhost:3000
  - http://localhost:8080/docs

checks:
  dirty_git: true              # warn on uncommitted changes

hooks:
  pre_launch:
    - docker compose up -d
  post_launch:
    - echo "ready!"
  pre_stop:
    - docker compose down
```

## Key Behaviors

- **`on init`** scans the current directory for Cargo.toml, package.json, pyproject.toml, or go.mod and generates a config automatically.
- **`extends:`** merges a base config — current values override, missing fields are inherited.
- **`env:`** on each pane injects `export KEY=VALUE` before the command. Values are shell-escaped.
- **Hooks** run as `sh -c` commands. If any hook fails, the operation is aborted.
- **Logs**: tmux uses `tmux capture-pane`; iTerm tees output to `~/.on/logs/`.
- **Port detection** extracts ports from browser URLs and pane commands, warns on conflicts before launch.
- **Process tracking** records PIDs in `~/.on/state/<project>.json` for clean stop/restart.

## When helping users

1. Use `on init` when setting up a new project — it auto-detects the stack.
2. Use `on clone` to duplicate configs for similar projects.
3. Use `extends: base` to share editor/browser/hooks across projects.
4. Use `env:` for secrets or per-environment config instead of .env files.
5. Use `hooks.pre_launch` for infrastructure (docker, databases) that must start first.
