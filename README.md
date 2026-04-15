# on

A CLI tool to restore your full dev environment with one command.

Stop manually opening terminals, editors, and browsers every time you start working — `on` does it all from a single YAML config.

## Install

**macOS (Homebrew):**

```bash
brew tap rickyshin93/tap
brew install rickyshin93/tap/on
```

**From source (macOS / Linux):**

```bash
cargo install --path .
```

## Quick Start

```bash
# Create a project config
on new myproject

# Edit the config
on edit myproject

# Launch the project
on myproject

# See all projects
on list

# Stop the project
on stop myproject

# Stop all projects
on stop --all

# Fuzzy select (no args)
on
```

## Configuration

Configs live in `~/.on/<project>.yaml`:

```yaml
name: myproject
terminal:
  type: tmux    # iterm | tmux (default: iterm on macOS, tmux on Linux)
  layout: vertical  # vertical (default) | grid
  max_panes_per_tab: 4  # max panes per tab (default 4, range 2-8)
  panes:
    - name: frontend
      dir: ~/projects/myproject/frontend
      cmd: npm run dev
    - name: backend
      dir: ~/projects/myproject/backend
      cmd: watchexec -e py python main.py
    - name: git
      dir: ~/projects/myproject
editor:
  cmd: cursor  # default: code
  folders:
    - ~/projects/myproject/frontend
    - ~/projects/myproject/backend
  # workspace: ~/.on/myproject.code-workspace
browser:
  - http://localhost:3000
  - https://github.com/me/myproject
```

## Features

- **Terminal Panes** — iTerm2 (macOS) or tmux (macOS/Linux), with auto-naming
- **Layouts** — `vertical` (side-by-side, default) or `grid` (tiled)
- **Multi-Tab** — Automatically splits panes across tabs when exceeding `max_panes_per_tab` (default 4)
- **Editor** — Opens configured editor with folders or workspace file
- **Browser** — Opens URLs in default browser
- **Port Conflict Detection** — Auto-detects ports from URLs/commands, warns on conflicts
- **Git Status** — Warns about uncommitted changes before launch
- **Process Tracking** — Tracks PIDs for clean `on stop`
- **Fuzzy Select** — Run `on` with no args to pick a project

## Requirements

- macOS or Linux
- [tmux](https://github.com/tmux/tmux) and/or [iTerm2](https://iterm2.com/) (macOS)

## License

[MIT](LICENSE)
