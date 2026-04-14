# on

A macOS CLI tool to restore your full dev environment with one command.

Stop manually opening terminals, editors, and browsers every time you start working — `on` does it all from a single YAML config.

## Install

```bash
brew tap rickyshin93/tap
brew install rickyshin93/tap/on
```

Or build from source:

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
iterm:
  layout: vertical  # vertical (default) | grid
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
browser:
  - http://localhost:3000
  - https://github.com/me/myproject
```

## Features

- **iTerm2 Panes** — Opens a tab per project, splits panes with auto-naming `[project] pane`
- **Layouts** — `vertical` (side-by-side, default) or `grid` (2x2)
- **Editor** — Opens configured editor (`code`, `cursor`, etc.) with project folders
- **Browser** — Opens URLs in default browser
- **Port Conflict Detection** — Auto-detects ports from URLs/commands, warns on conflicts
- **Git Status** — Warns about uncommitted changes before launch
- **Process Tracking** — Tracks PIDs for clean `on stop`
- **Fuzzy Select** — Run `on` with no args to pick a project

## Requirements

- macOS
- [iTerm2](https://iterm2.com/)

## License

[MIT](LICENSE)
