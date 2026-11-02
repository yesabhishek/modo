//! Source annotations, found with the same ignore-aware engine as project search.
use crate::{
    files,
    search::{self, Query},
};
use gpui_kit::component::{Rope, highlighter::SyntaxHighlighter};
use grep_matcher::Matcher as _;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub const TAGS: &[&str] = &["TODO", "FIXME", "BUG", "HACK", "NOTE", "XXX"];

#[derive(Clone, Debug)]
pub struct Todo {
    pub key: String,
    pub path: PathBuf,
    pub relative: String,
    pub line: u32,
    pub tag: String,
    pub text: String,
}

pub fn scan(root: &Path, cancel: &AtomicBool) -> Result<(Vec<Todo>, bool), String> {
    let query = Query {
        text: r"(?i)(?:^|//|#|/\*|<!--|--|;|\*)[ \t]*#?(?:TODO|FIXME|BUG|HACK|NOTE|XXX)\b".into(),
        regex: true,
        ..Query::default()
    };
    let results = search::search(root, &query, cancel)?;
    let matcher = search::matcher(&query)?;
    let mut todos = Vec::new();
    let mut occurrences = HashMap::<String, usize>::new();
    for file in results.files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // Search supplies candidate files; inspect their original source so a
        // tag inside a string literal cannot masquerade as a source comment.
        let mut source = String::new();
        let Ok(input) = std::fs::File::open(&file.path) else {
            continue;
        };
        const MAX_SOURCE: u64 = 4 * 1024 * 1024;
        if input
            .take(MAX_SOURCE + 1)
            .read_to_string(&mut source)
            .is_err()
            || source.len() as u64 > MAX_SOURCE
        {
            continue;
        }
        let language = files::language_for(&file.path);
        let mut syntax = SyntaxHighlighter::new(language);
        if !syntax.update(
            None,
            &Rope::from_str(&source),
            Some(Duration::from_millis(100)),
        ) {
            continue;
        }
        let tree = syntax.tree();
        let plain = matches!(language, "text" | "markdown");
        let candidate_lines: HashSet<_> = file.lines.iter().map(|line| line.line).collect();
        let mut offset = 0;
        for (row, raw) in source.split_inclusive('\n').enumerate() {
            let line = row as u32 + 1;
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if candidate_lines.contains(&line) {
                let _ = matcher.find_iter(raw.as_bytes(), |matched| {
                    let mut end = raw.len();
                    if !plain && let Some(tree) = tree {
                        let mut node = tree.root_node().descendant_for_byte_range(
                            offset + matched.start(),
                            offset + matched.end(),
                        );
                        let mut comment = None;
                        while let Some(current) = node {
                            if current.kind().contains("comment") {
                                comment = Some(current);
                                break;
                            }
                            node = current.parent();
                        }
                        let Some(comment) = comment else {
                            return true;
                        };
                        end = end.min(comment.end_byte().saturating_sub(offset));
                    } else if !raw[..matched.start()].trim().is_empty() {
                        // Unknown grammars and plain text accept annotations at
                        // the start of a line, without guessing inside code.
                        return true;
                    }
                    let annotation = &raw[matched.start()..end];
                    let upper = annotation.to_ascii_uppercase();
                    let Some((start, tag)) = TAGS
                        .iter()
                        .filter_map(|tag| upper.find(tag).map(|start| (start, *tag)))
                        .min_by_key(|(start, _)| *start)
                    else {
                        return true;
                    };
                    let text = annotation[start + tag.len()..]
                        .trim()
                        .trim_start_matches(':')
                        .trim()
                        .trim_end_matches("*/")
                        .trim_end_matches("-->")
                        .trim()
                        .to_owned();
                    let identity = format!("{}:{tag}:{text}", file.relative);
                    let occurrence = occurrences.entry(identity.clone()).or_default();
                    *occurrence += 1;
                    let key = blake3::hash(format!("{identity}:{occurrence}").as_bytes())
                        .to_hex()
                        .to_string();
                    todos.push(Todo {
                        key,
                        path: file.path.clone(),
                        relative: file.relative.to_string(),
                        line,
                        tag: tag.into(),
                        text,
                    });
                    false
                });
            }
            offset += raw.len();
        }
    }
    Ok((todos, results.limited))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strings_are_not_tasks_but_real_inline_and_block_comments_are() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            r###"fn main() {
    let text = "// TODO: fake string";
    let quoted = "\\\" // FIXME: still a string";
    let raw = r#"
// TODO: fake raw-string task
"#;
    let count = 1; // TODO: actual inline task
    let example = "// TODO: fake"; // FIXME: actual comment after a string
    /* TODO: actual block task */ let after = 0;
    /*
     * NOTE: actual multiline task
     */
}
"###,
        )
        .unwrap();
        let (tasks, _) = scan(dir.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.text.as_str())
                .collect::<Vec<_>>(),
            [
                "actual inline task",
                "actual comment after a string",
                "actual block task",
                "actual multiline task"
            ]
        );
    }

    #[test]
    fn python_multiline_strings_do_not_create_tasks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            r###"text = """
# TODO: fake docstring task
"""
value = "# TODO: fake string"
# TODO: real Python task
"###,
        )
        .unwrap();
        let (tasks, _) = scan(dir.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "real Python task");
    }

    #[test]
    fn finds_annotations_skips_ignored_files_and_preserves_identity_when_lines_move() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".ignore"), "secret.rs\n").unwrap();
        std::fs::write(dir.path().join("secret.rs"), "// TODO: secret").unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "// #TODO: ship it\n// FIXME(owner): handle error\nlet todoist = 0;\n// TODO: ship it\n").unwrap();
        let (before, _) = scan(dir.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(before.len(), 3);
        assert_eq!(before[0].text, "ship it");
        assert_eq!(before[1].tag, "FIXME");
        assert_ne!(before[0].key, before[2].key);
        std::fs::write(&path, "\n// #TODO: ship it\n// FIXME(owner): handle error\nlet todoist = 0;\n// TODO: ship it\n").unwrap();
        let (after, _) = scan(dir.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(before[0].key, after[0].key);
        assert_eq!(after[0].line, 2);
    }
}
