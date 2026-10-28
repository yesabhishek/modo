//! Project-wide text search on ripgrep's engine, and fuzzy path matching.
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Debug, Clone)]
pub struct LineMatch {
    /// One-based line number.
    pub line: u32,
    /// The line, trimmed and capped for display.
    pub text: String,
    /// Byte ranges into `text`.
    pub ranges: Vec<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct FileMatches {
    pub path: PathBuf,
    pub relative: Arc<str>,
    pub lines: Vec<LineMatch>,
}

#[derive(Debug, Default)]
pub struct Results {
    pub files: Vec<FileMatches>,
    pub total: usize,
    pub limited: bool,
}

pub const MAX_MATCHES: usize = 5_000;
const MAX_LINE: usize = 240;
const MAX_FILE: u64 = 4 * 1024 * 1024;

pub fn matcher(query: &Query) -> Result<RegexMatcher, String> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .case_smart(!query.case_sensitive)
        .case_insensitive(false)
        .word(query.whole_word)
        .fixed_strings(!query.regex);
    builder.build(&query.text).map_err(|e| e.to_string())
}

pub fn search(root: &Path, query: &Query, cancel: &AtomicBool) -> Result<Results, String> {
    if query.text.is_empty() {
        return Ok(Results::default());
    }
    let matcher = matcher(query)?;
    let files = Mutex::new(Vec::new());
    let total = AtomicUsize::new(0);
    let limited = AtomicBool::new(false);
    crate::files::walker(root).build_parallel().run(|| {
        let matcher = matcher.clone();
        let files = &files;
        let total = &total;
        let limited = &limited;
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .build();
        Box::new(move |entry| {
            use ignore::WalkState;
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if total.load(Ordering::Relaxed) >= MAX_MATCHES {
                limited.store(true, Ordering::Relaxed);
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file())
                || entry.metadata().map_or(true, |m| m.len() > MAX_FILE)
            {
                return WalkState::Continue;
            }
            let mut lines = Vec::new();
            let _ = searcher.search_path(
                &matcher,
                entry.path(),
                UTF8(|line_number, line| {
                    lines.push(display_line(&matcher, line_number as u32, line));
                    Ok(lines.len() < 200)
                }),
            );
            if !lines.is_empty() {
                total.fetch_add(lines.len(), Ordering::Relaxed);
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map(crate::files::relative_str)
                    .unwrap_or_else(|_| entry.path().to_string_lossy().into());
                if let Ok(mut files) = files.lock() {
                    files.push(FileMatches {
                        path: entry.into_path(),
                        relative,
                        lines,
                    });
                }
            }
            WalkState::Continue
        })
    });
    let mut files = files.into_inner().unwrap_or_default();
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(Results {
        total: files.iter().map(|f| f.lines.len()).sum(),
        files,
        limited: limited.into_inner(),
    })
}

fn display_line(matcher: &RegexMatcher, line: u32, raw: &str) -> LineMatch {
    let raw = raw.trim_end_matches(['\n', '\r']);
    let mut ranges = Vec::new();
    let _ = matcher.find_iter(raw.as_bytes(), |m| {
        ranges.push(m.start()..m.end());
        ranges.len() < 32
    });
    // Trim leading indentation, then keep the first match visible.
    let indent = raw.len() - raw.trim_start().len();
    let first = ranges.first().map_or(indent, |r| r.start);
    let mut start = if first.saturating_sub(indent) > MAX_LINE / 2 {
        first - MAX_LINE / 3
    } else {
        indent
    };
    while !raw.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + MAX_LINE).min(raw.len());
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    let text = raw[start..end].to_owned();
    let ranges = ranges
        .into_iter()
        .filter(|r| r.start >= start && r.end <= end)
        .map(|r| r.start - start..r.end - start)
        .collect();
    LineMatch { line, text, ranges }
}

/// Fuzzy path matching for quick open, scored like a good file picker:
/// file-name hits beat directory hits, shorter paths win ties.
pub struct Fuzzy {
    matcher: nucleo_matcher::Matcher,
    buffer: Vec<char>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self {
            matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()),
            buffer: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    pub index: usize,
    /// Char indices into the candidate that matched.
    pub positions: Vec<u32>,
}

impl Fuzzy {
    pub fn rank(&mut self, query: &str, candidates: &[Arc<str>], limit: usize) -> Vec<FuzzyMatch> {
        use nucleo_matcher::{
            Utf32Str,
            pattern::{CaseMatching, Normalization, Pattern},
        };
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut found = Vec::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let haystack = Utf32Str::new(candidate, &mut self.buffer);
            if let Some(score) = pattern.score(haystack, &mut self.matcher) {
                let name_start = candidate.rfind('/').map_or(0, |i| i + 1);
                let bonus = if candidate[name_start..]
                    .to_lowercase()
                    .contains(&query.to_lowercase().replace(' ', ""))
                {
                    40
                } else {
                    0
                };
                found.push((index, score + bonus));
            }
        }
        found.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| candidates[a.0].len().cmp(&candidates[b.0].len()))
        });
        found.truncate(limit);
        found
            .into_iter()
            .map(|(index, _)| {
                let mut positions = Vec::new();
                let haystack = Utf32Str::new(&candidates[index], &mut self.buffer);
                pattern.indices(haystack, &mut self.matcher, &mut positions);
                positions.sort_unstable();
                positions.dedup();
                FuzzyMatch { index, positions }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_literal_and_regex_matches_and_skips_binary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "fn main() {\n    let total = 1; // total\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.bin"),
            [b't', b'o', b't', b'a', b'l', 0, 1],
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let query = Query {
            text: "total".into(),
            ..Query::default()
        };
        let results = search(dir.path(), &query, &cancel).unwrap();
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.files[0].lines[0].line, 2);
        assert_eq!(results.files[0].lines[0].ranges.len(), 2);
        assert!(results.files[0].lines[0].text.starts_with("let total"));
        let regex = Query {
            text: r"fn \w+".into(),
            regex: true,
            ..Query::default()
        };
        assert_eq!(search(dir.path(), &regex, &cancel).unwrap().total, 1);
        let invalid = Query {
            text: "(".into(),
            regex: true,
            ..Query::default()
        };
        assert!(search(dir.path(), &invalid, &cancel).is_err());
    }

    #[test]
    fn fuzzy_prefers_file_names_and_short_paths() {
        let files: Vec<Arc<str>> = [
            "src/ui/workspace.rs",
            "docs/workspace-notes/readme.md",
            "src/main.rs",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let mut fuzzy = Fuzzy::default();
        let ranked = fuzzy.rank("worksp", &files, 10);
        assert_eq!(ranked[0].index, 0);
        let ranked = fuzzy.rank("main", &files, 10);
        assert_eq!(ranked[0].index, 2);
        assert!(!ranked[0].positions.is_empty());
    }
}
