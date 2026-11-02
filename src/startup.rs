//! Small, bounded startup diagnostics, written off the UI thread.
use std::{fs, io::Write};

pub fn log(message: String) {
    if std::env::var_os("MODO_TEMPORARY_SESSION").is_some() {
        return;
    }
    let Some(dir) = crate::settings::config_dir() else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("startup.log");
    // Keep the last run's diagnostics when rotating a log, never grow forever.
    if fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let previous = dir.join("startup.previous.log");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&path, previous);
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let elapsed = crate::STARTED
            .get()
            .map(|start| start.elapsed())
            .unwrap_or_default();
        let _ = writeln!(
            file,
            "Modo {} +{elapsed:?}: {message}",
            crate::brand::VERSION
        );
    }
}
