//! Read-only repository queries. Git runs off the UI thread with bounded output.
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_OUTPUT: u64 = 4 * 1024 * 1024;
/// Review patches cover a whole branch, so they get a larger, still bounded, budget.
const MAX_PATCH: u64 = 24 * 1024 * 1024;
/// Untracked files are shown as additions; skip anything that is clearly not source.
const MAX_UNTRACKED_FILE: u64 = 1024 * 1024;
const MAX_UNTRACKED_FILES: usize = 400;
/// Git's well-known empty tree, used as the base of unborn repositories.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const MAX_CHANGES: usize = 10_000;

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}
impl std::fmt::Display for Worktree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.branch == "Detached" {
            write!(
                f,
                "Detached · {}",
                self.path.file_name().unwrap_or_default().to_string_lossy()
            )
        } else {
            f.write_str(&self.branch)
        }
    }
}
impl PartialEq for Worktree {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for Worktree {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub index: char,
    pub working: char,
}
impl Change {
    pub fn badge(&self) -> &'static str {
        if self.index == 'U'
            || self.working == 'U'
            || matches!((self.index, self.working), ('A', 'A') | ('D', 'D'))
        {
            "!"
        } else if self.index == '?' || self.index == 'A' {
            "A"
        } else if self.index == 'D' || self.working == 'D' {
            "D"
        } else if self.index == 'R' || self.working == 'R' {
            "R"
        } else {
            "M"
        }
    }
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub root: PathBuf,
    pub branch: String,
    pub worktrees: Vec<Worktree>,
    pub changes: Vec<Change>,
    pub limited: bool,
}
impl Repository {
    pub fn change_for_path(&self, absolute: &Path) -> Option<&Change> {
        let relative = absolute.strip_prefix(&self.root).ok()?;
        self.changes
            .binary_search_by(|change| change.path.as_path().cmp(relative))
            .ok()
            .map(|index| &self.changes[index])
    }
}

struct Output {
    success: bool,
    bytes: Vec<u8>,
    error: String,
}
fn run<I, S>(root: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_capped(root, args, MAX_OUTPUT)
}

fn run_capped<I, S>(root: &Path, args: I, cap: u64) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // Files avoid pipe deadlocks and let us cap output while a child is running.
    let mut stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("--literal-pathspecs")
        .args(["-c", "color.ui=false", "-c", "core.quotepath=false"])
        .args(["-c", "core.fsmonitor=false"])
        .current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().map_err(|e| e.to_string())?)
        .stderr(stderr.try_clone().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot run Git. Install Git and make it available on PATH: {e}"))?;
    let started = Instant::now();
    let status = loop {
        let too_large = stdout.metadata().map(|m| m.len() > cap).unwrap_or(false)
            || stderr.metadata().map(|m| m.len() > cap).unwrap_or(false);
        if too_large || started.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if too_large {
                "Git output is too large to show"
            } else {
                "Git query timed out. Refresh to try again."
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    stdout.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    stderr.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    stdout
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > cap {
        return Err("Git output is too large to show".into());
    }
    let mut error = Vec::new();
    stderr
        .take(8192)
        .read_to_end(&mut error)
        .map_err(|e| e.to_string())?;
    Ok(Output {
        success: status.success(),
        bytes,
        error: String::from_utf8_lossy(&error).trim().to_owned(),
    })
}

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub fn inspect(folder: &Path) -> Result<Option<Repository>, String> {
    let top = run(folder, ["rev-parse", "--show-toplevel"])?;
    if !top.success {
        if top.error.contains("not a git repository") {
            return Ok(None);
        }
        return Err(top.error);
    }
    let root = path_from_bytes(top.bytes.strip_suffix(b"\n").unwrap_or(&top.bytes));
    let root = dunce::canonicalize(&root).unwrap_or(root);
    let branch = run(&root, ["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = if branch.success {
        String::from_utf8_lossy(&branch.bytes).trim().to_owned()
    } else {
        let head = run(&root, ["rev-parse", "--short", "HEAD"])?;
        format!("Detached {}", String::from_utf8_lossy(&head.bytes).trim())
    };
    let status = run(
        &root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !status.success {
        return Err(status.error);
    }
    let (mut changes, limited) = parse_status(&status.bytes)?;
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let trees = run(&root, ["worktree", "list", "--porcelain", "-z"])?;
    if !trees.success {
        return Err(trees.error);
    }
    let worktrees = parse_worktrees(&trees.bytes);
    Ok(Some(Repository {
        root,
        branch,
        changes,
        worktrees,
        limited,
    }))
}

fn parse_status(bytes: &[u8]) -> Result<(Vec<Change>, bool), String> {
    let mut records = bytes.split(|b| *b == 0);
    let mut changes = Vec::new();
    for_record(&mut records, &mut changes)?;
    let limited = changes.len() > MAX_CHANGES;
    changes.truncate(MAX_CHANGES);
    Ok((changes, limited))
}
fn for_record<'a>(
    records: &mut impl Iterator<Item = &'a [u8]>,
    changes: &mut Vec<Change>,
) -> Result<(), String> {
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned an invalid status record".into());
        }
        let index = record[0] as char;
        let working = record[1] as char;
        let original = if matches!(index, 'R' | 'C') || matches!(working, 'R' | 'C') {
            Some(path_from_bytes(
                records
                    .next()
                    .ok_or("Git rename is missing its original path")?,
            ))
        } else {
            None
        };
        changes.push(Change {
            path: path_from_bytes(&record[3..]),
            original,
            index,
            working,
        });
        if changes.len() > MAX_CHANGES {
            break;
        }
    }
    Ok(())
}
fn parse_worktrees(bytes: &[u8]) -> Vec<Worktree> {
    let mut result: Vec<Worktree> = Vec::new();
    for field in bytes.split(|b| *b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            let path = path_from_bytes(path);
            result.push(Worktree {
                path: dunce::canonicalize(&path).unwrap_or(path),
                branch: "Detached".into(),
            });
        } else if let Some(branch) = field.strip_prefix(b"branch refs/heads/")
            && let Some(tree) = result.last_mut()
        {
            tree.branch = String::from_utf8_lossy(branch).into_owned();
        }
    }
    result.retain(|tree| tree.path.is_dir());
    result
}

/// What a review compares the working tree against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewScope {
    /// Everything not yet committed: staged, unstaged, and untracked files.
    Uncommitted,
    /// Everything this branch changed since it left `base`, committed or not.
    Branch(String),
}

#[derive(Debug, Clone)]
pub struct ReviewPatch {
    pub patch: String,
    /// Untracked files that were too large or too many to include.
    pub skipped: usize,
}

/// The branch agents usually fork from: the remote default, else main/master.
pub fn default_base(root: &Path) -> Option<String> {
    let remote = run(
        root,
        [
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .ok()?;
    if remote.success {
        let name = String::from_utf8_lossy(&remote.bytes).trim().to_owned();
        if !name.is_empty() {
            return Some(name);
        }
    }
    ["main", "master", "trunk", "develop"]
        .into_iter()
        .find_map(|name| {
            let found = run(
                root,
                [
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{name}"),
                ],
            )
            .ok()?;
            found.success.then(|| name.to_owned())
        })
}

/// Local and remote branches to compare against, most recently committed first.
pub fn branches(root: &Path) -> Vec<String> {
    let Ok(out) = run(
        root,
        [
            "for-each-ref",
            "--sort=-committerdate",
            "--count=200",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    ) else {
        return Vec::new();
    };
    if !out.success {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.bytes)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("refs/heads/")
                .or_else(|| line.strip_prefix("refs/remotes/"))
        })
        // `origin/HEAD` only points at another remote branch.
        .filter(|name| !name.is_empty() && !name.ends_with("/HEAD"))
        .map(str::to_owned)
        .collect()
}

fn resolve(root: &Path, rev: &str) -> Option<String> {
    let out = run(
        root,
        [
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .ok()?;
    out.success
        .then(|| String::from_utf8_lossy(&out.bytes).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// One unified patch for every change in scope, ready for the review surface.
pub fn review_patch(root: &Path, scope: ReviewScope) -> Result<ReviewPatch, String> {
    let base = match &scope {
        ReviewScope::Uncommitted => resolve(root, "HEAD").unwrap_or_else(|| EMPTY_TREE.to_owned()),
        ReviewScope::Branch(base) => {
            let head = resolve(root, "HEAD").ok_or("This branch has no commits yet.")?;
            let base_commit =
                resolve(root, base).ok_or_else(|| format!("Cannot find branch {base}."))?;
            let merge_base = run(root, ["merge-base", &base_commit, &head])?;
            if !merge_base.success {
                return Err(format!("{base} and this branch share no history."));
            }
            String::from_utf8_lossy(&merge_base.bytes).trim().to_owned()
        }
    };
    let tracked = run_capped(
        root,
        [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--unified=3",
            &base,
            "--",
        ],
        MAX_PATCH,
    )?;
    if !tracked.success {
        return Err(tracked.error);
    }
    let mut patch = String::from_utf8_lossy(&tracked.bytes).into_owned();
    let untracked = run(root, ["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut skipped = 0;
    let mut included = 0;
    for raw in untracked.bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let relative = path_from_bytes(raw);
        let metadata = std::fs::symlink_metadata(root.join(&relative));
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(u64::MAX);
        if size > MAX_UNTRACKED_FILE
            || included >= MAX_UNTRACKED_FILES
            || patch.len() as u64 > MAX_PATCH
        {
            skipped += 1;
            continue;
        }
        let Ok(metadata) = metadata else { continue };
        // Built here rather than with `git diff --no-index` per file: hundreds
        // of Git processes take seconds on Windows.
        let (mode, content) = if metadata.file_type().is_symlink() {
            let Ok(target) = std::fs::read_link(root.join(&relative)) else {
                continue;
            };
            ("120000", target.to_string_lossy().into_owned().into_bytes())
        } else {
            let Ok(content) = std::fs::read(root.join(&relative)) else {
                continue;
            };
            let mode = if is_executable(&metadata) {
                "100755"
            } else {
                "100644"
            };
            (mode, content)
        };
        added_file_patch(&mut patch, &String::from_utf8_lossy(raw), mode, &content);
        included += 1;
    }
    Ok(ReviewPatch { patch, skipped })
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// Append the patch Git prints for a new file: every line added, or a binary marker.
fn added_file_patch(patch: &mut String, path: &str, mode: &str, content: &[u8]) {
    use std::fmt::Write as _;
    let old = quote_path("a/", path);
    let new = quote_path("b/", path);
    let _ = write!(patch, "diff --git {old} {new}\nnew file mode {mode}\n");
    if content.is_empty() {
        return;
    }
    // Git's own test: a NUL byte near the start means binary.
    if content[..content.len().min(8000)].contains(&0) {
        let _ = writeln!(patch, "Binary files /dev/null and {new} differ");
        return;
    }
    let text = String::from_utf8_lossy(content);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let lines = body.split('\n').count();
    let range = if lines == 1 {
        "1".to_owned()
    } else {
        format!("1,{lines}")
    };
    // Git ends a header path containing a space with a tab, for patch(1).
    let tab = if new.contains(' ') { "\t" } else { "" };
    let _ = write!(
        patch,
        "--- /dev/null\n+++ {new}{tab}\n@@ -0,0 +{range} @@\n"
    );
    for line in body.split('\n') {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    if !text.ends_with('\n') {
        patch.push_str("\\ No newline at end of file\n");
    }
}

/// Quote a path the way Git does in patch headers when it holds special characters.
fn quote_path(prefix: &str, path: &str) -> String {
    let needs_quotes = path
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control());
    if !needs_quotes {
        return format!("{prefix}{path}");
    }
    let mut quoted = format!("\"{prefix}");
    for c in path.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            c if c.is_control() => {
                let mut buf = [0; 4];
                for byte in c.encode_utf8(&mut buf).bytes() {
                    quoted.push_str(&format!("\\{byte:03o}"));
                }
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        let output = run(root, args).unwrap();
        assert!(output.success, "{}", output.error);
    }
    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        git(
            dir.path(),
            &["config", "user.email", "modo-test@example.invalid"],
        );
        git(dir.path(), &["config", "user.name", "Modo test"]);
        std::fs::write(dir.path().join("file.txt"), "original\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(
            dir.path(),
            &["-c", "commit.gpgsign=false", "commit", "-m", "Initial"],
        );
        dir
    }
    fn commit(root: &Path, message: &str) {
        git(root, &["add", "-A"]);
        git(
            root,
            &["-c", "commit.gpgsign=false", "commit", "-m", message],
        );
    }
    #[test]
    fn folders_without_git_are_supported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(inspect(dir.path()).unwrap().is_none());
    }
    #[test]
    fn uncommitted_review_covers_staged_unstaged_and_unusual_untracked_paths() {
        let dir = repository();
        std::fs::write(dir.path().join("file.txt"), "staged\n").unwrap();
        git(dir.path(), &["add", "file.txt"]);
        std::fs::write(dir.path().join("file.txt"), "working\n").unwrap();
        let unusual = if cfg!(windows) {
            "-[中文]file.txt"
        } else {
            "-[中文] file.txt"
        };
        std::fs::write(dir.path().join(unusual), "new file\n").unwrap();
        let repo = inspect(dir.path()).unwrap().unwrap();
        assert_eq!(repo.branch, "main");
        let review = review_patch(&repo.root, ReviewScope::Uncommitted).unwrap();
        assert!(review.patch.contains("-original"));
        assert!(review.patch.contains("+working"));
        assert!(review.patch.contains("+new file"));
        assert_eq!(review.skipped, 0);
        let new_file = repo
            .changes
            .iter()
            .find(|c| c.path == Path::new(unusual))
            .unwrap();
        assert_eq!(new_file.badge(), "A");
    }
    #[test]
    fn branch_review_includes_commits_since_the_base_and_local_edits() {
        let dir = repository();
        git(dir.path(), &["switch", "-c", "agent/feature"]);
        std::fs::write(dir.path().join("agent.rs"), "fn agent() {}\n").unwrap();
        commit(dir.path(), "Agent work");
        std::fs::write(dir.path().join("file.txt"), "edited after commit\n").unwrap();
        assert_eq!(default_base(dir.path()).as_deref(), Some("main"));
        let review = review_patch(dir.path(), ReviewScope::Branch("main".into())).unwrap();
        assert!(review.patch.contains("+fn agent() {}"));
        assert!(review.patch.contains("+edited after commit"));
        let uncommitted = review_patch(dir.path(), ReviewScope::Uncommitted).unwrap();
        assert!(!uncommitted.patch.contains("fn agent"));
    }
    #[test]
    fn any_branch_can_be_the_review_base() {
        let dir = repository();
        git(dir.path(), &["switch", "-c", "dev"]);
        std::fs::write(dir.path().join("dev.txt"), "dev\n").unwrap();
        commit(dir.path(), "Dev work");
        git(dir.path(), &["switch", "-c", "agent/feature"]);
        std::fs::write(dir.path().join("agent.txt"), "agent\n").unwrap();
        commit(dir.path(), "Agent work");
        let branches = branches(dir.path());
        assert_eq!(branches.len(), 3);
        assert!(branches.iter().any(|b| b == "dev"));
        assert!(branches.iter().any(|b| b == "agent/feature"));
        let since_dev = review_patch(dir.path(), ReviewScope::Branch("dev".into())).unwrap();
        assert!(since_dev.patch.contains("+agent"));
        assert!(!since_dev.patch.contains("+dev"));
        let since_main = review_patch(dir.path(), ReviewScope::Branch("main".into())).unwrap();
        assert!(since_main.patch.contains("+dev"));
        let missing = review_patch(dir.path(), ReviewScope::Branch("gone".into()));
        assert_eq!(missing.unwrap_err(), "Cannot find branch gone.");
    }
    #[test]
    fn renames_deletions_and_existing_worktrees_are_discovered() {
        let dir = repository();
        git(dir.path(), &["mv", "file.txt", "renamed.txt"]);
        let repo = inspect(dir.path()).unwrap().unwrap();
        let renamed = &repo.changes[0];
        assert_eq!(renamed.badge(), "R");
        assert_eq!(renamed.original.as_deref(), Some(Path::new("file.txt")));
        let parent = tempfile::tempdir().unwrap();
        let tree = parent.path().join("review");
        git(
            dir.path(),
            &["worktree", "add", "-b", "review", tree.to_str().unwrap()],
        );
        let repo = inspect(dir.path()).unwrap().unwrap();
        assert!(
            repo.worktrees
                .iter()
                .any(|w| w.branch == "review" && w.path == dunce::canonicalize(&tree).unwrap())
        );
        std::fs::remove_file(dir.path().join("renamed.txt")).unwrap();
        let repo = inspect(dir.path()).unwrap().unwrap();
        assert_eq!(repo.changes[0].working, 'D');
        assert!(
            review_patch(&repo.root, ReviewScope::Uncommitted)
                .unwrap()
                .patch
                .contains("deleted file")
        );
    }
    #[test]
    fn untracked_files_patch_exactly_like_git_diff_no_index() {
        let dir = repository();
        let cases: &[(&str, &[u8])] = &[
            ("lines.txt", b"one\ntwo\n"),
            ("single.txt", b"only\n"),
            ("no-newline.txt", b"first\nlast"),
            ("crlf.txt", b"a\r\nb\r\n"),
            ("blank-lines.txt", b"\n\nx\n"),
            ("binary.bin", &[0, 1, 2, 3]),
            ("empty.txt", b""),
            ("dir/with space.md", b"# title\n"),
        ];
        std::fs::create_dir(dir.path().join("dir")).unwrap();
        for (path, content) in cases {
            std::fs::write(dir.path().join(path), content).unwrap();
        }
        for (path, content) in cases {
            let git = run(
                dir.path(),
                [
                    "-c",
                    "core.autocrlf=false",
                    "diff",
                    "--no-ext-diff",
                    "--no-index",
                    "--",
                    "/dev/null",
                    path,
                ],
            )
            .unwrap();
            // Git adds blob ids, which the review never reads.
            let expected: String = String::from_utf8_lossy(&git.bytes)
                .split_inclusive('\n')
                .filter(|line| !line.starts_with("index "))
                .collect();
            let mut ours = String::new();
            added_file_patch(&mut ours, path, "100644", content);
            assert_eq!(ours, expected, "{path}");
        }
        let review = review_patch(dir.path(), ReviewScope::Uncommitted).unwrap();
        let files = gpui_kit::component::diff::DiffFile::parse(&review.patch).unwrap();
        assert_eq!(files.len(), cases.len());
        assert_eq!(quote_path("a/", "say \"hi\".txt"), r#""a/say \"hi\".txt""#);
    }
    #[test]
    fn unborn_repositories_review_against_the_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        std::fs::write(dir.path().join("first.txt"), "first\n").unwrap();
        git(dir.path(), &["add", "."]);
        std::fs::write(dir.path().join("binary.bin"), [0, 1, 2, 3]).unwrap();
        let repo = inspect(dir.path()).unwrap().unwrap();
        let review = review_patch(&repo.root, ReviewScope::Uncommitted).unwrap();
        assert!(review.patch.contains("+first"));
        assert!(review.patch.contains("Binary files"));
    }
}
