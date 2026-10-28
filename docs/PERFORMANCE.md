# Performance

Measured on an Apple M4 Pro, macOS 27.2, release build (`cargo build --release`), October 2026. Each run used `--temporary-session` defaults and was sampled with `footprint` and `ps`.

| Scenario | First frame | Physical footprint | RSS | Idle CPU (10 s) |
| --- | --- | --- | --- | --- |
| Small project, one file open | 0.44 s | 65 MB | 101 MB | 0.1% |
| This repository, no file open | 0.30 s | 65 MB | 101 MB | 0.0% |

| Artifact | Size |
| --- | --- |
| Release binary (arm64) | 48 MB |
| macOS DMG (arm64) | 12.5 MB |
| macOS ZIP (arm64) | 10.0 MB |

Most of the binary is Tree-sitter grammar tables. They are memory-mapped and only paged in when a file of that language opens, so they cost disk space, not memory.

## How to measure

```bash
cargo build --release
MODO_LOG_STARTUP=1 target/release/modo path/to/project   # prints time to first frame
footprint $(pgrep -n modo)                               # macOS memory
```

Idle CPU should stay at zero: Modo only redraws on input, file changes, or terminal output.
