# Changelog

## 0.3.2 (2026-11-02)

- Added Midnight and Black themes, plus optional, theme-aware project gradients across the app to distinguish workspaces. Each project can toggle its gradient independently.
- Replaced the native About alert with a themed, keyboard-accessible dialog containing the app icon, version, publisher card, and clickable source and release links.
- Redesigned Settings > About with a compact app header and separate Publisher, Updates, Resources, and startup-diagnostic controls.
- Terminals support full-window mode and nested horizontal and vertical splits.
- Removed the blue outline around terminal panes. Split dividers and the active terminal tab still show the layout and focus.
- Review individual recent commits or a whole branch. Files can be marked reviewed, with progress remembered per project and reset when their diff changes.
- The file tree honors .ignore and .gitignore, hides ignored entries by default, and can show them with muted labels. The hidden-files preference now works.
- Tab menus support closing all tabs, all but one, and tabs to the left or right, with sequential unsaved-edit prompts and conflict-safe saves.
- Open Folder is available on the empty project page. A compact, scrollable recent-folder picker shows folder names and abbreviated paths, with direct switching and a separate new-window button.
- A To-dos sidebar finds TODO, FIXME, BUG, HACK, NOTE, and XXX source annotations, filters by tag, jumps to their source, and remembers completed tasks. Rows stay within the sidebar width, and language parsing excludes tags inside string literals and test fixtures.
- Session file reads run off the UI thread, editors restore incrementally, and bounded startup diagnostics are saved in startup.log.
- Packaged installs check for releases in the background at startup, verify SHA-256 checksums, and install updates. Windows waits until Modo exits; protected Linux installs use the system installer.
- Publisher details now identify Abhishek Choudhury (www.abhishekchoudhury.com).

## 0.2.2 (2026-10-28)

- **Faster terminal on Windows.** The installer now ships Windows Terminal's ConPTY (`conpty.dll` and `OpenConsole.exe`), which is quicker and more faithful than the console host built into Windows. PowerShell starts without its banner, and PowerShell 7 is preferred when installed. Drawing the terminal also does less work per frame on every platform.
- **Tab completion works.** Tab and Shift+Tab now reach the shell instead of moving focus. On Windows and Linux, Ctrl+C copies a selection and otherwise interrupts; on Windows, Ctrl+V pastes.
- **Normal paths on Windows.** Projects open at `C:\...` rather than `\\?\C:\...`, which made PowerShell show a `FileSystem::` prompt, broke path completion, and refused to run local scripts such as `.venv\Scripts\activate`.
- **Terminal profiles.** Pick the default shell in Settings > Terminal, or open any installed shell from the menu beside **+**: PowerShell, Windows PowerShell, Command Prompt, Git Bash, and WSL on Windows; zsh, bash, fish, Nushell, and PowerShell on macOS and Linux. A custom command (with quoted paths) adds a Custom profile.
- **Compare with any branch.** The review's compare menu, in the sidebar and beside the branch name, offers uncommitted changes or everything since any local or remote branch, most recent first. The choice is remembered per project.
- **Cleaner change list.** Long file names truncate instead of running into the line counts, and the list draws only visible rows, so branches with thousands of changed files stay smooth.
- **Quicker reviews with new files.** Untracked files are diffed in-process rather than one Git process each.

## 0.2.1 (2026-10-28)

First public release. Modo is a ground-up rebuild of the earlier Aardvark IDX / Kite prototype.

- **New name and identity.** Modo, with a monochrome "M with a check" icon and Notion-inspired Light, Dark, Paper, and Graphite themes.
- **GPU rendering.** Rebuilt on GPUI and GPUI Kit (Metal on macOS, DirectX on Windows, Vulkan on Linux) for smooth scrolling and native text rendering.
- **Review.** One virtualized diff of a whole branch since its base, or only uncommitted work, in unified or split view with word-level changes. Worktree switching from the sidebar.
- **Live updates.** Files, the tree, Git status, and the review refresh while agents edit.
- **Editor.** Tree-sitter highlighting for over 25 languages, folding, multiple cursors, find and replace, conflict-safe saves.
- **Markdown preview.** `⌘⇧V` (Ctrl+Shift+V) or the eye button flips a `.md` file between source and a clean rendered page, with ```` ```mermaid ```` blocks drawn as diagrams in the active theme.
- **Search.** Fuzzy go-to-file, project search on ripgrep's engine, and a command palette with `>`, `:`, and `@` modes.
- **Terminal.** Native PTY with tabs, true color, and the user's login shell.
- **Install.** macOS DMG with a `modo` command, Windows installer with "Open with Modo", and a Linux `.deb` plus tarball with a desktop entry.
