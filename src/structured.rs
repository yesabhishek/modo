//! Bounded, lossless structured text parsing. Models contain data, never UI elements.
use std::{
    collections::HashSet,
    ops::Range,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_ITEMS: usize = 250_000;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Json,
    Csv(u8),
}
impl Kind {
    pub fn for_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "json" | "geojson" => Some(Self::Json),
            "csv" => Some(Self::Csv(b',')),
            "tsv" => Some(Self::Csv(b'\t')),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    pub key: Option<Range<usize>>,
    pub value: Range<usize>,
    pub parent: Option<usize>,
    pub depth: usize,
    pub index: usize,
    pub children: Vec<usize>,
}
#[derive(Debug)]
pub enum Model {
    Json {
        source: String,
        nodes: Vec<Node>,
    },
    Csv {
        rows: Vec<Vec<String>>,
        columns: usize,
        delimiter: u8,
    },
}

fn check(cancel: &AtomicBool, items: usize) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled".into());
    }
    if items >= MAX_ITEMS {
        return Err(
            "Structured view is limited to 250,000 values. Use Source to read or search this file."
                .into(),
        );
    }
    Ok(())
}

pub fn parse(kind: Kind, text: &str, cancel: &AtomicBool) -> Result<Model, String> {
    if text.len() as u64 > crate::document::MAX_FILE_BYTES {
        return Err(
            "Structured view supports files up to 10 MiB. Use Source to inspect this file.".into(),
        );
    }
    check(cancel, 0)?;
    match kind {
        Kind::Json => {
            // Validate without deserializing numbers or collecting objects: duplicate
            // keys, original ordering, and arbitrarily precise numbers stay intact.
            serde_json::from_str::<serde::de::IgnoredAny>(text)
                .map_err(|e| format!("Invalid JSON: {e}"))?;
            let mut parser = JsonParser {
                text,
                pos: 0,
                nodes: Vec::new(),
                cancel,
            };
            parser.value(None, None, 0, 0)?;
            Ok(Model::Json {
                source: text.into(),
                nodes: parser.nodes,
            })
        }
        Kind::Csv(delimiter) => parse_csv(text, delimiter, cancel),
    }
}

struct JsonParser<'a> {
    text: &'a str,
    pos: usize,
    nodes: Vec<Node>,
    cancel: &'a AtomicBool,
}
impl JsonParser<'_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.pos += 1;
        }
    }
    fn string(&mut self) -> Range<usize> {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.text.len() {
            match self.text.as_bytes()[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    break;
                }
                _ => self.pos += 1,
            }
        }
        start..self.pos
    }
    fn value(
        &mut self,
        key: Option<Range<usize>>,
        parent: Option<usize>,
        depth: usize,
        index: usize,
    ) -> Result<usize, String> {
        check(self.cancel, self.nodes.len())?;
        if depth >= MAX_DEPTH {
            return Err("JSON nesting exceeds 128 levels. Use Source to inspect this file.".into());
        }
        self.space();
        let start = self.pos;
        let id = self.nodes.len();
        self.nodes.push(Node {
            key,
            value: start..start,
            parent,
            depth,
            index,
            children: Vec::new(),
        });
        let first = self.text.as_bytes()[self.pos];
        if first == b'{' || first == b'[' {
            self.pos += 1;
            self.space();
            let end = if first == b'{' { b'}' } else { b']' };
            while self.text.as_bytes()[self.pos] != end {
                let key = if first == b'{' {
                    self.space();
                    let key = self.string();
                    self.space();
                    self.pos += 1; // colon, already validated
                    Some(key)
                } else {
                    None
                };
                let child = self.value(key, Some(id), depth + 1, self.nodes[id].children.len())?;
                self.nodes[id].children.push(child);
                self.space();
                if self.text.as_bytes()[self.pos] == b',' {
                    self.pos += 1;
                    self.space();
                }
            }
            self.pos += 1;
        } else if first == b'"' {
            self.string();
        } else {
            while self
                .text
                .as_bytes()
                .get(self.pos)
                .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(b))
            {
                self.pos += 1;
            }
        }
        self.nodes[id].value.end = self.pos;
        Ok(id)
    }
}

/// Strict CSV quoting; preserves empty cells, ragged rows, spaces, and embedded newlines.
fn parse_csv(text: &str, delimiter: u8, cancel: &AtomicBool) -> Result<Model, String> {
    let bytes = text.as_bytes();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut pos = 0;
    let mut cells = 0;
    let mut columns = 0;
    while pos < bytes.len() {
        check(cancel, cells)?;
        let mut value = Vec::new();
        if bytes[pos] == b'"' {
            pos += 1;
            let mut closed = false;
            while pos < bytes.len() {
                if bytes[pos] == b'"' {
                    pos += 1;
                    if bytes.get(pos) == Some(&b'"') {
                        value.push(b'"');
                        pos += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    value.push(bytes[pos]);
                    pos += 1;
                }
            }
            if !closed {
                return Err(format!(
                    "CSV record {} has an unclosed quoted field.",
                    rows.len() + 1
                ));
            }
            if bytes
                .get(pos)
                .is_some_and(|b| *b != delimiter && *b != b'\n' && *b != b'\r')
            {
                return Err(format!(
                    "Unexpected text after a closing quote in CSV record {}.",
                    rows.len() + 1
                ));
            }
        } else {
            let start = pos;
            while pos < bytes.len()
                && bytes[pos] != delimiter
                && bytes[pos] != b'\n'
                && bytes[pos] != b'\r'
            {
                if bytes[pos] == b'"' {
                    return Err(format!(
                        "Quote inside an unquoted field in CSV record {}.",
                        rows.len() + 1
                    ));
                }
                pos += 1;
            }
            value.extend_from_slice(&bytes[start..pos]);
        }
        row.push(String::from_utf8(value).map_err(|e| e.to_string())?);
        cells += 1;
        if bytes.get(pos) == Some(&delimiter) {
            pos += 1;
            if pos == bytes.len() {
                check(cancel, cells)?;
                row.push(String::new());
            } else {
                continue;
            }
        } else if bytes.get(pos) == Some(&b'\r') {
            pos += 1;
            if bytes.get(pos) == Some(&b'\n') {
                pos += 1;
            }
        } else if bytes.get(pos) == Some(&b'\n') {
            pos += 1;
        }
        columns = columns.max(row.len());
        rows.push(std::mem::take(&mut row));
    }
    Ok(Model::Csv {
        rows,
        columns,
        delimiter,
    })
}

fn decoded(raw: &str) -> String {
    if raw.starts_with('"') {
        serde_json::from_str(raw).unwrap_or_else(|_| raw.into())
    } else {
        raw.into()
    }
}

/// One linear scan, including hidden JSON descendants. Ancestors supply context.
pub fn visible(
    model: &Model,
    expanded: &HashSet<usize>,
    query: &str,
    headers: bool,
    cancel: &AtomicBool,
) -> Vec<usize> {
    let query = query.to_lowercase();
    match model {
        Model::Json { source, nodes } => {
            let mut included = vec![false; nodes.len()];
            if query.is_empty() {
                for (id, node) in nodes.iter().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        return Vec::new();
                    }
                    included[id] = node
                        .parent
                        .is_none_or(|p| included[p] && expanded.contains(&p));
                }
            } else {
                for (id, node) in nodes.iter().enumerate().rev() {
                    if cancel.load(Ordering::Relaxed) {
                        return Vec::new();
                    }
                    included[id] |= node.key.as_ref().is_some_and(|key| {
                        decoded(&source[key.clone()])
                            .to_lowercase()
                            .contains(&query)
                    }) || (node.children.is_empty()
                        && decoded(&source[node.value.clone()])
                            .to_lowercase()
                            .contains(&query));
                    if included[id]
                        && let Some(parent) = node.parent
                    {
                        included[parent] = true;
                    }
                }
            }
            included
                .into_iter()
                .enumerate()
                .filter_map(|(id, yes)| yes.then_some(id))
                .collect()
        }
        Model::Csv { rows, .. } => rows
            .iter()
            .enumerate()
            .skip(usize::from(headers))
            .take_while(|_| !cancel.load(Ordering::Relaxed))
            .filter_map(|(id, row)| {
                (query.is_empty() || row.iter().any(|cell| cell.to_lowercase().contains(&query)))
                    .then_some(id)
            })
            .collect(),
    }
}

pub fn format(
    kind: Kind,
    text: &str,
    indent: usize,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let model = parse(kind, text, cancel)?;
    let mut out = String::with_capacity(text.len());
    fn bound(out: &str) -> Result<(), String> {
        if out.len() as u64 > crate::document::MAX_FILE_BYTES {
            Err("Formatted text would exceed the 10 MiB editing limit.".into())
        } else {
            Ok(())
        }
    }
    match model {
        Model::Json { source, nodes } => {
            fn write(
                id: usize,
                source: &str,
                nodes: &[Node],
                indent: usize,
                out: &mut String,
                cancel: &AtomicBool,
            ) -> Result<(), String> {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let node = &nodes[id];
                if node.children.is_empty() {
                    out.push_str(&source[node.value.clone()]);
                } else {
                    let object = source.as_bytes()[node.value.start] == b'{';
                    out.push(if object { '{' } else { '[' });
                    out.push('\n');
                    for (ix, child) in node.children.iter().enumerate() {
                        out.extend(std::iter::repeat_n(' ', (node.depth + 1) * indent));
                        if let Some(key) = &nodes[*child].key {
                            out.push_str(&source[key.clone()]);
                            out.push_str(": ");
                        }
                        write(*child, source, nodes, indent, out, cancel)?;
                        if ix + 1 < node.children.len() {
                            out.push(',');
                        }
                        out.push('\n');
                    }
                    out.extend(std::iter::repeat_n(' ', node.depth * indent));
                    out.push(if object { '}' } else { ']' });
                }
                bound(out)
            }
            write(0, &source, &nodes, indent.clamp(1, 8), &mut out, cancel)?;
            out.push('\n');
        }
        Model::Csv {
            rows, delimiter, ..
        } => {
            for row in rows {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                for (ix, cell) in row.iter().enumerate() {
                    if ix > 0 {
                        out.push(delimiter as char);
                    }
                    if cell
                        .bytes()
                        .any(|b| b == delimiter || b"\"\r\n".contains(&b))
                        || (row.len() == 1 && cell.is_empty())
                    {
                        out.push('"');
                        out.push_str(&cell.replace('"', "\"\""));
                        out.push('"');
                    } else {
                        out.push_str(cell);
                    }
                    bound(&out)?;
                }
                out.push('\n');
            }
        }
    }
    bound(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cancel() -> AtomicBool {
        AtomicBool::new(false)
    }
    #[test]
    fn json_format_preserves_duplicates_order_precision_and_strings() {
        let input = r#"{"z":1.234567890123456789,"z":999999999999999999999999,"a":"quote\\\" and \\n","x":[],"b":true}"#;
        let formatted = format(Kind::Json, input, 2, &cancel()).unwrap();
        assert!(formatted.starts_with("{\n  \"z\": 1.234567890123456789,"));
        assert_eq!(formatted.matches("\"z\":").count(), 2);
        assert!(formatted.contains("999999999999999999999999"));
        assert_eq!(
            format(Kind::Json, &formatted, 2, &cancel()).unwrap(),
            formatted
        );
    }
    #[test]
    fn json_search_finds_collapsed_descendants_and_decodes_escapes() {
        let model = parse(
            Kind::Json,
            r#"{"outer":{"hidden":"caf\u00e9"},"other":0}"#,
            &cancel(),
        )
        .unwrap();
        assert_eq!(
            visible(&model, &HashSet::new(), "", true, &cancel()),
            vec![0]
        );
        assert_eq!(
            visible(&model, &HashSet::new(), "CAFÉ", true, &cancel()),
            vec![0, 1, 2]
        );
        assert!(visible(&model, &HashSet::new(), "missing", true, &cancel()).is_empty());
    }
    #[test]
    fn malformed_and_overdeep_json_are_rejected_without_changes() {
        for input in ["", "{", "[1,]", "true false", "{\"x\": NaN}"] {
            assert!(parse(Kind::Json, input, &cancel()).is_err());
        }
        let deep = format!("{}0{}", "[".repeat(150), "]".repeat(150));
        assert!(parse(Kind::Json, &deep, &cancel()).is_err());
    }
    #[test]
    fn csv_roundtrip_quotes_newlines_unicode_ragged_rows_and_empty_cells() {
        for input in [
            "name,value\r\n\"café, x\",\"a\"\"b\"\r\n\"multi\nline\",\n",
            "a,b,",
            ",",
            "\n",
            "\"\"",
            "a\nb,c\n",
            "",
            "  a , b  \n",
        ] {
            let formatted = format(Kind::Csv(b','), input, 2, &cancel()).unwrap();
            let original = parse(Kind::Csv(b','), input, &cancel()).unwrap();
            let result = parse(Kind::Csv(b','), &formatted, &cancel()).unwrap();
            match (original, result) {
                (Model::Csv { rows: a, .. }, Model::Csv { rows: b, .. }) => {
                    assert_eq!(a, b, "{input:?}")
                }
                _ => panic!(),
            }
        }
    }
    #[test]
    fn malformed_csv_is_rejected_and_search_includes_all_columns() {
        for input in ["\"unclosed", "ab\"cd", "\"x\"oops,y"] {
            assert!(parse(Kind::Csv(b','), input, &cancel()).is_err());
        }
        let model = parse(
            Kind::Csv(b'\t'),
            "name\tvalue\na\tHELLO\nb\tother",
            &cancel(),
        )
        .unwrap();
        assert_eq!(
            visible(&model, &HashSet::new(), "hello", true, &cancel()),
            vec![1]
        );
        assert_eq!(
            visible(&model, &HashSet::new(), "name", false, &cancel()),
            vec![0]
        );
    }
    #[test]
    fn dense_inputs_and_cancelled_work_are_bounded() {
        assert!(parse(Kind::Csv(b','), &",".repeat(MAX_ITEMS), &cancel()).is_err());
        assert!(parse(Kind::Json, "{}", &AtomicBool::new(true)).is_err());
        let model = parse(Kind::Csv(b','), &"a,b\n".repeat(50_000), &cancel()).unwrap();
        assert_eq!(
            visible(&model, &HashSet::new(), "b", false, &cancel()).len(),
            50_000
        );
    }
}
