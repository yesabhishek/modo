# Contributing

Modo stays small on purpose: a place to read, review, and lightly edit code. Changes that make that faster, calmer, or more reliable are welcome. For anything large, open an issue first.

## Setup

Install [Rust](https://rustup.rs); `rust-toolchain.toml` picks the compiler. On Linux, install the libraries listed in the README. Then:

```bash
cargo run -- path/to/project
```

## Before you open a pull request

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For visible changes, attach before and after screenshots. You can render them without touching the screen:

```bash
cargo run --features capture -- --capture shot.png --scene review path/to/repo
```

Scenes: `new-file`, `editor`, `preview` (markdown, via `MODO_CAPTURE_FILE`), `review`, `review-split`, `palette`, `commands`, `search`, `settings`, `settings-terminal`, `about`, `settings-about`, `todos`, `review-progress`, `terminal-split`, `terminal-full`, `window-color`, `project-gradient`, `project-gradient-terminal`, `live`, and `tour` (writes a folder of frames for the README GIF). `MODO_CAPTURE_THEME=light|dark|paper|graphite|midnight|black` picks a theme. `MODO_CAPTURE_CLICK=x,y` clicks a point (window coordinates) before the shot, for example to open a menu.

## Style

- Keep the interface quiet: one accent color, soft hover fills, no borders where spacing will do.
- Write UI copy and docs in plain words. No emojis or em dashes.
- Match the surrounding code; comments explain why, not what.

To exercise file creation, autosave, manual saving, and conflict protection in a disposable project:

```bash
mkdir -p /tmp/modo-editing-smoke
MODO_TEMPORARY_SESSION=1 cargo run --features capture -- --capture /tmp/modo-editing-smoke.png --scene editing-smoke /tmp/modo-editing-smoke
```

The project must not already contain `smoke.txt`. The scenario asserts disk contents and dirty state and exits unsuccessfully if a check fails.

The `data-smoke` scene creates `data-smoke.json` and `data-smoke.csv` in a disposable project and checks native search, lossless formatting, undo, and CSV autosave. Run it with a fresh project folder and `MODO_TEMPORARY_SESSION=1`.

The `save-race-smoke` scene checks edits made during an in-flight save and autosave in background tabs. It creates `save-race.txt` and `other.txt` in a disposable project.
