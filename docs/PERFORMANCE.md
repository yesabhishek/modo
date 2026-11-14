# Performance

Measured on an Apple M4 Pro, macOS 27.2, release build (`scripts/package-macos.sh`), November 2026. Each figure is the middle of three launches, sampled with `footprint` and `ps` a few seconds after launch.

| Scenario | First frame | Physical footprint | RSS |
| --- | --- | --- | --- |
| Small project, no file open | 0.35 s | 81 MB | 112 MB |
| Saved session with 28 source files open | 0.31 s | 111 MB | 114 MB |

The same two launches in 0.4.0 took 0.44 s and 0.44 s, and the session with 28 tabs used 239 MB. The difference comes from three changes: launch no longer lists every installed font, the window is created while the launch request is still being read, and a restored tab is only a path until it is shown.

"First frame" is the first frame with the project in it, not the empty window that precedes it.

| Artifact | Size |
| --- | --- |
| Release binary (arm64) | 51 MB |
| macOS DMG (arm64) | 13.8 MB |
| macOS ZIP (arm64) | 11.3 MB |

Most of the binary is Tree-sitter grammar tables. They are memory-mapped and only paged in when a file of that language opens, so they cost disk space, not memory.

## What keeps it light

- At most twelve editors are mounted. Tabs beyond that keep their place in the bar and are read again when shown; a tab with unsaved edits is never unloaded.
- A save does not re-index the project, refresh the file tree, or reload the tab that was saved.
- The review diff is re-read when it is on screen, not after every change.
- Checks for files changed on disk run off the UI thread, and a file whose size and time are unchanged is not read.
- The file watcher drops build output, dependencies, and Git's object store on its own thread and checks each path there; more than 2,000 changes in one burst are handled as a single rescan.
- At most one `git status` runs at a time; changes that arrive meanwhile fold into one follow-up. Auto save waits for typing to settle before asking Git.
- Settings and the session are written on a background thread and flushed on quit.
- Go to file ranks projects of 5,000 files or more off the UI thread.
- Mermaid diagrams are laid out and rasterised on a background thread, capped at 8 megapixels each and 96 MB in all, and uploaded as raw pixels.

## How to measure

```bash
cargo build --release
MODO_LOG_STARTUP=1 target/release/modo path/to/project   # prints time to first frame
footprint $(pgrep -n modo)                               # macOS memory
```

Idle CPU should stay at zero: Modo only redraws on input, file changes, or terminal output.

## Startup diagnostics

Each normal launch writes its first-frame time to `startup.log` in Modo's configuration directory. The log rotates at 64 KiB, retaining `startup.previous.log`; temporary sessions do not write it. Session restoration reads only the active tab's file, on a background thread, so it does not block the first frame. Update checks start after the first frame and run in the background.
