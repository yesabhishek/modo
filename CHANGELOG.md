# Changelog

## 0.6.0 (2026-11-15)

- **Opening a folder never runs its code.** Git filters, signature checks, file system monitors, and lazy fetches named in a project's own `.git/config` are switched off when Modo reads status and diffs. Filters from your global config, such as Git LFS, keep working.
- **Safer markdown.** Links open only web and mail addresses, and files inside the project. Other URL schemes, network shares, and paths that climb out of the project are refused. Images load only from the project; web images never load, so a document cannot report that it was opened.
- **Signed updates.** Each release's checksum list is signed with the publisher's key, and Modo installs an update only when the signature matches. The automatic check runs at most once a day.
- **Terminal clipboard.** Programs in the terminal can copy (tmux, Neovim over SSH) but never read the clipboard, and the status bar says when one copies.
- **Snappier with busy agents.** Build output and Git's object store are filtered out before file events reach the window, a burst of thousands of changes is read once, Git status never runs twice at a time, and auto save no longer runs Git after every pause. Settings are written in the background. Go to file ranks large projects off the UI thread and lists your open tabs first.
- **Lighter.** Mermaid diagrams are capped in size and memory and skip a PNG round trip. Five duplicate crates are gone.
- **Review tells the truth.** When the base branch shares no history with yours, the review shows uncommitted changes and says why, the same way on the Review tab, the overview, and the Changes panel. The status bar counts "uncommitted" files.
- **Quieter sidebar.** One row of tabs (Files, Search, Changes, To-dos) and a settings button replace five stacked rows. The Changes panel no longer repeats the branch and compare menu. Text sizes and corners follow one scale throughout.
- **Auto save timing.** Settings > Editor offers Off, after 0.25 s, after 1 s, or when leaving the file. Every settings row is one line.

## 0.5.0 (2026-11-10)

- **Check for updates.** Settings > About has a Check for updates button, also in the app menu and the command palette. It reports whether a newer release is out, downloads it with a progress figure in the status bar, and offers Restart now when it is installed. Source builds are pointed at the release page instead. The automatic check at launch is unchanged and no longer shows a message when the network is down.
- **Updating while Modo is open.** Modo notices when the app on disk is no longer the one that is running, whether the updater, an installer, or `scripts/install.sh` replaced it, and shows Restart to update in the status bar. Restarting asks about unsaved files first and reopens the new version. The Windows installer now waits for Modo to be closed instead of failing on a file that is in use, and relaunches Modo after an update you asked to restart for. `scripts/install.sh` swaps the app in place so there is never a moment without it.
- **Edits survive any kind of quit.** With auto save on, unsaved edits are written when the app is quit from the Dock, at logout, or by an installer, which previously skipped the save prompt.
- **Quicker auto save.** Files save a quarter of a second after you stop typing (was three quarters), and immediately when you switch tabs or leave the window. A save no longer triggers a re-index of the project, a refresh of the file tree, or a reload of the file it just wrote.
- **An overview when no file is open.** The empty page now shows what the branch changed: files, lines added and removed, review progress, and the busiest files with a bar for the size and balance of each change. Click a file to jump to it in the review.
- **Better markdown.** Markdown files open rendered, with the source one click away. Front matter appears as a small table, images beside the file load, links to project files open in Modo (with `#L12` line anchors) and other links open in the browser. Mermaid diagrams are drawn in the background instead of holding up the page.
- **Tab bar.** A long row of tabs scrolls inside the bar instead of running over the terminal button, the active tab is always scrolled into view, and a menu at the end lists every open tab.
- **Settings, redone.** Five short pages of grouped one-line rows with a title that stays put. The editor font is a menu of the installed monospace families. Scrolling works at any window size and no longer moves the page behind the dialog.
- **Tidier interface.** Slightly smaller bars, rows, and controls throughout.
- **Faster and lighter.** Launch no longer lists every installed font, which took about a tenth of a second, and the window is created while the launch request is still being read. Restoring a session reads only the tab you left on; the others load when first shown. With many tabs open, at most twelve keep their editor in memory and the rest are reread when you return to them (never a tab with unsaved edits). Checks for files changed on disk run off the UI thread and skip unchanged files without reading them. With auto save on, the review is re-read when it is next shown rather than after every pause in typing.

## 0.4.0 (2026-11-05)

- JSON opens in a native expandable tree; CSV and TSV open in native tables with a header toggle, cell copying, and column pages for wide files. Source editing stays one click away.
- Search structured views with Cmd+F / Ctrl+F. JSON search includes collapsed descendants and decoded string values; CSV search scans every column. Rows are virtualized, parsing and filtering run off the UI thread, and stale searches are cancelled.
- Format JSON and CSV/TSV with the Format button, command palette, or Alt+Shift+F. Formatting is undoable, preserves JSON key order, duplicate keys, and exact number text, and retains CSV cell content and quoting semantics. Malformed data is reported without modification. CSV saves preserve significant trailing spaces even when whitespace trimming is enabled.
- Structured parsing is bounded to 10 MiB and 250,000 values, with depth limits, source fallback, and no new runtime dependencies.

- Create files from the Files sidebar toolbar or a folder's right-click menu. Enter a file name and press Enter or Create; the new file opens immediately. Existing files are never overwritten, and creation errors stay beside the input.
- Auto save is enabled by default and saves edited files after a short pause in typing, including background tabs. Settings > Editor can turn it off; manual Save and Save All remain available.
- Automatic saves use the existing conflict-safe atomic save path, preserve edits made during a save, and stop retrying failed revisions until another edit or manual save.

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
