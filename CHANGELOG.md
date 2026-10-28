# Changelog

## 0.2.0 (2026-10-28)

Modo is a ground-up rebuild of the earlier Aardvark IDX / Kite prototype.

- **New name and identity.** Modo, with a monochrome "M with a check" icon and Notion-inspired Light, Dark, Paper, and Graphite themes.
- **GPU rendering.** Rebuilt on GPUI and GPUI Kit (Metal on macOS, DirectX on Windows, Vulkan on Linux) for smooth scrolling and native text rendering.
- **Review.** One virtualized diff of a whole branch since its base, or only uncommitted work, in unified or split view with word-level changes. Worktree switching from the sidebar.
- **Live updates.** Files, the tree, Git status, and the review refresh while agents edit.
- **Editor.** Tree-sitter highlighting for over 25 languages, folding, multiple cursors, find and replace, conflict-safe saves.
- **Markdown preview.** `⌘⇧V` (Ctrl+Shift+V) or the eye button flips a `.md` file between source and a clean rendered page, with ```` ```mermaid ```` blocks drawn as diagrams in the active theme.
- **Search.** Fuzzy go-to-file, project search on ripgrep's engine, and a command palette with `>`, `:`, and `@` modes.
- **Terminal.** Native PTY with tabs, true color, and the user's login shell.
- **Install.** macOS DMG with a `modo` command, Windows installer with "Open with Modo", and a Linux `.deb` plus tarball with a desktop entry.
