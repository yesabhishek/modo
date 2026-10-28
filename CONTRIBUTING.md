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

Scenes: `editor`, `preview` (markdown, via `MODO_CAPTURE_FILE`), `review`, `review-split`, `palette`, `commands`, `search`, `settings`, `live`, and `tour` (writes a folder of frames for the README GIF). `MODO_CAPTURE_THEME=light|dark|paper|graphite` picks a theme.

## Style

- Keep the interface quiet: one accent color, soft hover fills, no borders where spacing will do.
- Write UI copy and docs in plain words. No emojis or em dashes.
- Match the surrounding code; comments explain why, not what.
