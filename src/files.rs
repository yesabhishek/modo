//! The project on disk: lazily read directories for the tree, and a parallel,
//! ignore-aware index of every file for quick open.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// Folders that are never worth walking for quick open or search.
pub const HEAVY_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".gradle",
    ".idea",
    "Pods",
    "DerivedData",
    ".turbo",
    ".cache",
];

pub fn read_directory(path: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" || name == ".DS_Store" {
            continue;
        }
        // Symlinked folders stay leaves so a link cycle cannot recurse forever.
        let directory = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(Entry {
            path: entry.path(),
            name,
            directory,
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(entries)
}

/// Case-insensitive ordering where `file2` sorts before `file10`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    a.next();
                }
                let mut nb = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    b.next();
                }
                let order = na
                    .trim_start_matches('0')
                    .len()
                    .cmp(&nb.trim_start_matches('0').len())
                    .then_with(|| na.cmp(&nb));
                if order.is_ne() {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order.is_ne() {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct FileIndex {
    /// Paths relative to the root with `/` separators, sorted.
    pub files: Vec<Arc<str>>,
    pub limited: bool,
}

pub const MAX_INDEXED_FILES: usize = 250_000;

struct Batch<'a> {
    local: Vec<Arc<str>>,
    out: &'a Mutex<Vec<Arc<str>>>,
}

impl Batch<'_> {
    fn flush(&mut self) {
        if let Ok(mut out) = self.out.lock() {
            out.append(&mut self.local);
        }
    }
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Walk the project on all cores, honoring .gitignore and skipping build output.
pub fn index_files(root: &Path, cancel: &AtomicBool) -> FileIndex {
    let found = Mutex::new(Vec::with_capacity(4096));
    let count = AtomicUsize::new(0);
    let limited = AtomicBool::new(false);
    walker(root).build_parallel().run(|| {
        let count = &count;
        let limited = &limited;
        // Each walker thread batches locally; the batch flushes when the
        // thread's visitor is dropped, so no tail of results is lost.
        let mut batch = Batch {
            local: Vec::with_capacity(512),
            out: &found,
        };
        Box::new(move |entry| {
            use ignore::WalkState;
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if entry.file_type().is_some_and(|t| t.is_file()) {
                if count.fetch_add(1, Ordering::Relaxed) >= MAX_INDEXED_FILES {
                    limited.store(true, Ordering::Relaxed);
                    return WalkState::Quit;
                }
                if let Ok(relative) = entry.path().strip_prefix(root) {
                    batch.local.push(relative_str(relative));
                }
                if batch.local.len() >= 512 {
                    batch.flush();
                }
            }
            WalkState::Continue
        })
    });
    let mut files = found.into_inner().unwrap();
    files.sort_unstable();
    files.dedup();
    FileIndex {
        files,
        limited: limited.into_inner(),
    }
}

pub fn walker(root: &Path) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .max_depth(Some(40))
        .threads(std::thread::available_parallelism().map_or(4, |n| n.get().min(8)))
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_some_and(|t| t.is_dir())
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| HEAVY_DIRS.contains(&name))
        });
    builder
}

pub fn relative_str(path: &Path) -> Arc<str> {
    let mut out = String::with_capacity(path.as_os_str().len());
    for (i, part) in path.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&part.as_os_str().to_string_lossy());
    }
    out.into()
}

/// A display path like `src/ui/app.rs` for anything inside `root`.
pub fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| relative_str(p).to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Language id for the editor's highlighter, from the file name.
pub fn language_for(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match name.as_str() {
        "makefile" | "gnumakefile" => return "make",
        "dockerfile" | ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zprofile" => {
            return "bash";
        }
        "cargo.lock" | "pipfile" => return "toml",
        _ => {}
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "py" | "pyi" | "pyw" => "python",
        "go" => "go",
        "json" | "jsonc" | "json5" | "geojson" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" | "mdx" => "markdown",
        "html" | "htm" | "xhtml" | "vue" => "html",
        "css" | "scss" | "less" => "css",
        "sh" | "bash" | "zsh" | "fish" | "env" => "bash",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "mm" | "m" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" | "rake" | "gemspec" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "sql" => "sql",
        "zig" => "zig",
        "diff" | "patch" => "diff",
        "mk" => "make",
        _ => "text",
    }
}

pub fn language_label(language: &str) -> &'static str {
    match language {
        "rust" => "Rust",
        "typescript" => "TypeScript",
        "tsx" => "TypeScript JSX",
        "javascript" => "JavaScript",
        "python" => "Python",
        "go" => "Go",
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" => "YAML",
        "markdown" => "Markdown",
        "html" => "HTML",
        "css" => "CSS",
        "bash" => "Shell",
        "c" => "C",
        "cpp" => "C++",
        "csharp" => "C#",
        "java" => "Java",
        "kotlin" => "Kotlin",
        "swift" => "Swift",
        "ruby" => "Ruby",
        "php" => "PHP",
        "lua" => "Lua",
        "sql" => "SQL",
        "zig" => "Zig",
        "diff" => "Diff",
        "make" => "Makefile",
        _ => "Plain Text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_respects_ignore_files_and_skips_build_trees() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        for i in 0..700 {
            fs::write(dir.path().join(format!("src/file{i}.rs")), "").unwrap();
        }
        fs::create_dir(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/large.js"), "").unwrap();
        let index = index_files(dir.path(), &AtomicBool::new(false));
        assert!(index.files.iter().any(|p| &**p == "src/file0.rs"));
        assert!(index.files.iter().any(|p| &**p == "src/file699.rs"));
        assert_eq!(
            index.files.iter().filter(|p| p.starts_with("src/")).count(),
            700
        );
        assert!(
            !index
                .files
                .iter()
                .any(|p| p.ends_with("secret.txt") || p.ends_with("large.js"))
        );
    }

    #[test]
    fn tree_sorts_folders_first_naturally_and_hides_git() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("z-dir")).unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        for name in ["b.rs", "A.rs", "file10.rs", "file2.rs"] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let entries = read_directory(dir.path()).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["z-dir", "A.rs", "b.rs", "file2.rs", "file10.rs"]
        );
        assert!(entries[0].directory);
    }

    #[test]
    fn languages_map_from_names_and_extensions() {
        assert_eq!(language_for(Path::new("src/main.rs")), "rust");
        assert_eq!(language_for(Path::new("Makefile")), "make");
        assert_eq!(language_for(Path::new("App.TSX")), "tsx");
        assert_eq!(language_for(Path::new("notes")), "text");
    }
}
