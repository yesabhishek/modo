//! Text files on disk: UTF-8 loading, conflict-safe atomic saves, and fingerprints.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub path: PathBuf,
    pub text: String,
    pub line_ending: LineEnding,
    pub bom: bool,
    pub fingerprint: blake3::Hash,
}

pub fn load(path: &Path) -> Result<LoadedFile, String> {
    let path =
        dunce::canonicalize(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular text file.".into());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("This file exceeds the 10 MiB editing limit.".into());
    }
    // Keep the limit effective even if another process grows the file after
    // the metadata check.
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(&path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Cannot read file: {e}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("This file exceeds the 10 MiB editing limit.".into());
    }
    if bytes.contains(&0) {
        return Err("Binary files are not supported.".into());
    }
    let fingerprint = blake3::hash(&bytes);
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let raw = std::str::from_utf8(if bom { &bytes[3..] } else { &bytes }).map_err(|_| {
        "Only UTF-8 text files are supported; the file was not modified.".to_owned()
    })?;
    let line_ending = if raw.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    Ok(LoadedFile {
        path,
        text: raw.replace("\r\n", "\n"),
        line_ending,
        bom,
        fingerprint,
    })
}

pub fn fingerprint(path: &Path) -> Result<Option<blake3::Hash>, String> {
    match fs::File::open(path) {
        Ok(mut file) => {
            let mut hasher = blake3::Hasher::new();
            hasher.update_reader(&mut file).map_err(|e| e.to_string())?;
            Ok(Some(hasher.finalize()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Write in the target directory, then atomically replace. Never truncate the original.
pub fn save(
    path: &Path,
    text: &str,
    line_ending: LineEnding,
    bom: bool,
    expected: Option<blake3::Hash>,
) -> Result<blake3::Hash, String> {
    if fingerprint(path)? != expected {
        return Err("File changed on disk. Reopen it or use Save As to keep both versions.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut bytes = if bom {
        vec![0xef, 0xbb, 0xbf]
    } else {
        Vec::new()
    };
    if line_ending == LineEnding::CrLf {
        bytes.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
    } else {
        bytes.extend_from_slice(text.as_bytes());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("Cannot save here: {e}"))?;
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.permissions().readonly() {
            return Err("This file is read-only. Use Save As to save a copy.".into());
        }
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temporary.write_all(&bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    if fingerprint(path)? != expected {
        return Err("File changed while saving; your original is intact.".into());
    }
    temporary
        .persist(path)
        .map_err(|e| format!("Cannot replace file: {}", e.error))?;
    Ok(blake3::hash(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_bom_crlf_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.rs");
        let bytes = b"\xef\xbb\xbffn main() {\r\n}\r\n";
        fs::write(&path, bytes).unwrap();
        let file = load(&path).unwrap();
        assert!(file.bom);
        assert_eq!(file.line_ending, LineEnding::CrLf);
        save(
            &path,
            &file.text,
            file.line_ending,
            file.bom,
            Some(file.fingerprint),
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let text = "हिन्दी 𝄞 café\n";
        let other = dir.path().join("unicode.txt");
        save(&other, text, LineEnding::Lf, false, None).unwrap();
        assert_eq!(load(&other).unwrap().text, text);
    }
    #[test]
    fn external_changes_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, "original").unwrap();
        let file = load(&path).unwrap();
        fs::write(&path, "external").unwrap();
        assert!(save(&path, "mine", LineEnding::Lf, false, Some(file.fingerprint)).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "external");
    }
    #[test]
    fn binary_invalid_encoding_and_large_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        for bytes in [vec![0, 1], vec![0xff, 0xfe]] {
            fs::write(&path, bytes).unwrap();
            assert!(load(&path).is_err());
        }
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(load(&path).is_err());
    }
}
