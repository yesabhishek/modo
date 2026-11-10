# Architecture

Modo is one Rust binary built on [GPUI](https://gpui.rs) (Zed's GPU UI framework) and GPUI Kit (components: code editor, diff view, inputs, notifications). There are no plugins, language servers, or background indexers.

## Layout

```
src/
  main.rs        CLI arguments, app startup, menus, open-folder events
  brand.rs       Name, bundle id, version: a rename touches one file
  theme.rs       Maps the user's theme choice onto assets/themes/modo.json
  settings.rs    Preferences and per-project session, saved as JSON
  document.rs    UTF-8 load, conflict-safe atomic save, fingerprints
  files.rs       Lazy directory reads, parallel ignore-aware file index
  search.rs      Project search (ripgrep crates) and fuzzy path ranking
  git.rs         Read-only Git: status, worktrees, review patches
  structured.rs  JSON and CSV parsing and lossless formatting
  todos.rs       TODO, FIXME, and friends, found in source comments
  updater.rs     Release checks, downloads, installs, and restart
  capture.rs     Offscreen screenshots and smoke tests (feature "capture")
  ui/
    workspace.rs Window root: project, tabs, file watching, actions
    sidebar.rs   Project switcher, navigation, file tree, search, changes
    editor.rs    An open file: editor state, saves, reloads
    preview.rs   Rendered markdown and Mermaid diagrams
    data_view.rs JSON tree and CSV table
    review.rs    Branch or uncommitted diff of the whole project
    terminal.rs  PTY sessions and the GPU-painted terminal grid
    palette.rs   Go to file, commands, line, and project switching
    welcome.rs   The first screen, and the change overview when no tab is open
    settings_view.rs, about.rs
```

## Key decisions

- **GPU rendering.** GPUI draws with Metal, DirectX, or Vulkan and shapes text with the platform's own font engine, so scrolling stays smooth and text looks native.
- **Virtualize everything long.** The file tree, search results, palette, and diff only build the rows on screen.
- **Keep the UI thread free.** Directory reads, indexing, search, Git, and saves run on background threads, and stale results are dropped by generation counters. The one exception is "Save All" while a window closes, which saves before the window goes away.
- **Review is Git's view of the work.** `git diff <merge-base>` plus untracked files, parsed into one diff. Agents usually commit as they go, so branch review is the default; uncommitted-only is one click away.
- **Follow the agent.** A recursive file watcher (FSEvents, inotify, ReadDirectoryChangesW) refreshes the tree, Git status, clean open files, and the review. Files with unsaved edits are never replaced; Modo shows a banner instead.
- **Saves are safe.** Writes go to a temporary file beside the target, then replace it, only if the file on disk still matches what was loaded.
- **A save is not an outside edit.** The watcher sees Modo's own saves too. A saved file whose size and time still match what the tab wrote, and the temporary file beside it, are ignored, so auto save does not re-index the project or reload the tab.
- **Tabs are cheap until shown.** A tab restored from the session is only a path; its file is read and its editor mounted when the tab is first selected. At most twelve editors stay mounted: beyond that, the clean tabs unused longest go back to being a path, keeping their place, cursor, and preview state. A tab with unsaved edits is never unloaded.
- **The diff is read when it is looked at.** Changes mark the review stale; it is re-read at once if the Review tab, the overview, or the change list is on screen, and otherwise when one of them next is.
- **No font listing at launch.** Listing every installed family costs about a tenth of a second on macOS. The editor font is resolved by asking the text system for one name at a time; the full list is read on a background thread only when Settings is opened.
- **Terminal.** alacritty_terminal runs the PTY and parses output; Modo paints the grid itself, one styled run per span. The shell starts at the panel's real size, as a direct login shell.
