//! Background release updates. Only packaged installs update themselves; source
//! builds and capture runs never replace a developer's binary.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use futures::{StreamExt, channel::mpsc};
use gpui_kit::{App, Global, SharedString};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    brand,
    settings::{self, Settings},
};

#[derive(Clone, Default)]
pub struct UpdateState {
    pub message: Option<SharedString>,
    /// Packages requiring the operating system's installer remain accessible.
    pub package: Option<PathBuf>,
}
impl Global for UpdateState {}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

pub fn init(cx: &mut App) {
    cx.set_global(UpdateState::default());
    if !Settings::get(cx).auto_update
        || cfg!(debug_assertions)
        || std::env::var_os("MODO_TEMPORARY_SESSION").is_some()
        || std::env::args_os().any(|arg| arg == "--capture")
    {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if !is_installed(&exe, std::env::consts::OS) {
        return;
    }
    let (tx, mut rx) = mpsc::unbounded::<UpdateState>();
    cx.spawn(async move |cx| {
        // Let the first frame and session restoration finish first.
        cx.background_executor().timer(Duration::from_secs(3)).await;
        if !cx.update(|cx| Settings::get(cx).auto_update) {
            return;
        }
        let worker = cx.background_executor().spawn(async move {
            if let Err(error) = check_and_install(&exe, &tx) {
                crate::startup::log(format!("update: {error}"));
                let _ = tx.unbounded_send(UpdateState {
                    message: Some(format!("Update check failed: {error}").into()),
                    package: None,
                });
            }
        });
        while let Some(state) = rx.next().await {
            cx.update(|cx| cx.set_global(state));
        }
        worker.await;
    })
    .detach();
}

fn is_installed(exe: &Path, os: &str) -> bool {
    if exe.components().any(|part| part.as_os_str() == "target") {
        return false;
    }
    match os {
        "macos" => exe.ends_with("Modo.app/Contents/MacOS/modo"),
        "windows" => exe.file_name().is_some_and(|name| name == "modo.exe"),
        "linux" => exe.file_name().is_some_and(|name| name == "modo"),
        _ => false,
    }
}

fn newer_version(release: &Release, current: &str) -> Result<Option<Version>, String> {
    let version =
        Version::parse(release.tag_name.trim_start_matches('v')).map_err(|e| e.to_string())?;
    let current = Version::parse(current).map_err(|e| e.to_string())?;
    Ok(
        (!release.draft && !release.prerelease && version.pre.is_empty() && version > current)
            .then_some(version),
    )
}

fn asset_names(version: &str, os: &str, arch: &str) -> Vec<String> {
    match (os, arch) {
        ("macos", "aarch64") => vec![
            format!("modo-{version}-macos-universal.zip"),
            format!("modo-{version}-macos-arm64.zip"),
        ],
        ("macos", "x86_64") => vec![
            format!("modo-{version}-macos-universal.zip"),
            format!("modo-{version}-macos-x64.zip"),
        ],
        ("windows", "x86_64") => vec![format!("modo-{version}-windows-x64-setup.exe")],
        ("linux", "x86_64") => vec![format!("modo-{version}-linux-x86_64.tar.gz")],
        _ => Vec::new(),
    }
}

struct UpdateLock(PathBuf);
impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn check_and_install(exe: &Path, tx: &mpsc::UnboundedSender<UpdateState>) -> Result<(), String> {
    let dir = settings::config_dir()
        .ok_or("No update directory is available")?
        .join("updates");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock_path = dir.join("checking.lock");
    // Recover a lock left by a crash; normal checks and downloads time out sooner.
    if fs::metadata(&lock_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(600))
    {
        let _ = fs::remove_file(&lock_path);
    }
    let Ok(lock_file) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    else {
        return Ok(());
    };
    drop(lock_file);
    let _lock = UpdateLock(lock_path);
    let stage = tempfile::Builder::new()
        .prefix("release-")
        .tempdir_in(&dir)
        .map_err(|e| e.to_string())?;
    let metadata_path = stage.path().join("release.json");
    let repo = brand::REPOSITORY
        .strip_prefix("https://github.com/")
        .ok_or("Invalid release repository")?;
    download(
        &format!("https://api.github.com/repos/{repo}/releases/latest"),
        &metadata_path,
        2 * 1024 * 1024,
        15,
    )?;
    let release: Release =
        serde_json::from_slice(&fs::read(metadata_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let Some(version) = newer_version(&release, brand::VERSION)? else {
        return Ok(());
    };
    let version = version.to_string();
    let mut names = asset_names(&version, std::env::consts::OS, std::env::consts::ARCH);
    if cfg!(target_os = "linux") && exe.starts_with("/usr") {
        names.insert(0, format!("modo_{version}_amd64.deb"));
    }
    let asset = names
        .iter()
        .find_map(|name| release.assets.iter().find(|asset| &asset.name == name))
        .ok_or("No update package matches this platform")?;
    validate_asset_url(
        &asset.browser_download_url,
        repo,
        &release.tag_name,
        &asset.name,
    )?;
    let expected = match asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
    {
        Some(digest) => digest.to_owned(),
        None => {
            let stem = asset
                .name
                .trim_end_matches("-setup.exe")
                .trim_end_matches(".tar.gz")
                .trim_end_matches(".zip");
            let checksum = release
                .assets
                .iter()
                .find(|candidate| candidate.name == format!("{stem}-SHA256SUMS.txt"))
                .or_else(|| {
                    release.assets.iter().find(|candidate| {
                        candidate.name == format!("modo-{version}-linux-SHA256SUMS.txt")
                    })
                })
                .ok_or("This release has no SHA-256 checksum")?;
            validate_asset_url(
                &checksum.browser_download_url,
                repo,
                &release.tag_name,
                &checksum.name,
            )?;
            let checksum_path = stage.path().join("SHA256SUMS.txt");
            download(
                &checksum.browser_download_url,
                &checksum_path,
                64 * 1024,
                15,
            )?;
            checksum_for(
                &fs::read_to_string(checksum_path).map_err(|e| e.to_string())?,
                &asset.name,
            )
            .ok_or("The package is missing from SHA256SUMS")?
        }
    };
    let _ = tx.unbounded_send(UpdateState {
        message: Some(format!("Downloading Modo {version}…").into()),
        package: None,
    });
    let package = stage.path().join(&asset.name);
    download(
        &asset.browser_download_url,
        &package,
        512 * 1024 * 1024,
        120,
    )?;
    verify_checksum(&package, &expected)?;
    if let Err(error) = install(exe, &package, &version, &dir) {
        crate::startup::log(format!("update installation: {error}"));
        let _ = tx.unbounded_send(UpdateState {
            message: Some(format!("Modo {version} downloaded; click to install. {error}").into()),
            package: Some(package.clone()),
        });
        let _ = stage.keep();
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        // The installer and its waiting helper must outlive this process.
        let _ = stage.keep();
    }
    let _ = tx.unbounded_send(UpdateState {
        message: Some(
            if cfg!(target_os = "windows") {
                format!("Modo {version} downloaded; installs after all Modo windows quit")
            } else {
                format!("Modo {version} installed; available next launch")
            }
            .into(),
        ),
        package: None,
    });
    Ok(())
}

fn validate_asset_url(url: &str, repo: &str, tag: &str, name: &str) -> Result<(), String> {
    if name.contains(['/', '\\'])
        || name == "."
        || name == ".."
        || url != format!("https://github.com/{repo}/releases/download/{tag}/{name}")
    {
        return Err("The update asset is outside the release repository".into());
    }
    Ok(())
}

fn download(url: &str, target: &Path, limit: u64, seconds: u64) -> Result<(), String> {
    let output = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "5",
            "--max-time",
            &seconds.to_string(),
            "--max-filesize",
            &limit.to_string(),
            "--user-agent",
            "Modo-Updater",
            "--output",
        ])
        .arg(target)
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Cannot download update: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(200)
            .collect());
    }
    Ok(())
}

fn checksum_for(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        (fields.next()?.trim_start_matches('*') == name).then(|| hash.to_owned())
    })
}

fn verify_checksum(path: &Path, expected: &str) -> Result<(), String> {
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid release checksum".into());
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let len = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    if format!("{:x}", hasher.finalize()) != expected.to_ascii_lowercase() {
        return Err("The update failed SHA-256 verification".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install(exe: &Path, package: &Path, version: &str, _updates: &Path) -> Result<(), String> {
    let app = exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or("Modo is not running from an app bundle")?;
    let parent = app.parent().ok_or("The app has no installation folder")?;
    // Work on the same volume so replacement and rollback are renames.
    let stage = tempfile::Builder::new()
        .prefix(".modo-update-")
        .tempdir_in(parent)
        .map_err(|e| format!("The installation folder is not writable: {e}"))?;
    run(Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(package)
        .arg(stage.path()))?;
    let candidate = stage.path().join("Modo.app");
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&candidate))?;
    for (key, expected) in [
        ("CFBundleIdentifier", brand::BUNDLE_ID),
        ("CFBundleShortVersionString", version),
    ] {
        let output = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Print :{key}")])
            .arg(candidate.join("Contents/Info.plist"))
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != expected {
            return Err(format!("The update bundle has an unexpected {key}"));
        }
    }
    let backup = stage.path().join("previous.app");
    fs::rename(app, &backup).map_err(|e| format!("Cannot move the previous app: {e}"))?;
    if let Err(error) = fs::rename(&candidate, app) {
        if fs::rename(&backup, app).is_err() {
            let recovery = stage.keep();
            return Err(format!(
                "Cannot install update: {error}; previous app preserved in {}",
                recovery.display()
            ));
        }
        return Err(format!(
            "Cannot install update: {error}; previous app restored"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install(exe: &Path, package: &Path, version: &str, _updates: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if package.extension().is_some_and(|ext| ext == "deb") {
        return Err("A system installation needs package-manager permission".into());
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(package)
        .arg("-C")
        .arg(stage.path()))?;
    let binary = stage
        .path()
        .join(format!("modo-{version}-linux-x86_64/modo"));
    let parent = exe.parent().ok_or("No installation folder")?;
    let mut replacement = tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("The installation folder requires permission; update Modo through your package manager: {e}"))?;
    std::io::copy(
        &mut fs::File::open(binary).map_err(|e| e.to_string())?,
        &mut replacement,
    )
    .map_err(|e| e.to_string())?;
    replacement
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    replacement
        .as_file()
        .sync_all()
        .map_err(|e| e.to_string())?;
    replacement.persist(exe).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn install(exe: &Path, package: &Path, _version: &str, _updates: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let dir = exe.parent().ok_or("No installation folder")?;
    let script = package
        .parent()
        .ok_or("No update staging folder")?
        .join("install.ps1");
    fs::write(&script, r#"param([int]$ParentId, [string]$Installer, [string]$InstallDir)
$ErrorActionPreference = 'Stop'
Wait-Process -Id $ParentId -ErrorAction SilentlyContinue
while (Get-Process -Name modo -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $InstallDir 'modo.exe') }) { Start-Sleep -Seconds 1 }
$process = Start-Process -FilePath $Installer -ArgumentList @('/S', ('/D=' + $InstallDir)) -Wait -PassThru
if ($process.ExitCode -ne 0) { throw ('Modo installer failed: ' + $process.ExitCode) }
Remove-Item -LiteralPath $Installer -Force
"#).map_err(|e| e.to_string())?;
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-ParentId")
        .arg(std::process::id().to_string())
        .arg("-Installer")
        .arg(package)
        .arg("-InstallDir")
        .arg(dir)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn install(_exe: &Path, _package: &Path, _version: &str, _updates: &Path) -> Result<(), String> {
    Err("Automatic updates are unavailable on this platform".into())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run(command: &mut Command) -> Result<(), String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(240)
            .collect());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn release(version: &str) -> Release {
        Release {
            tag_name: version.into(),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }
    }
    #[test]
    fn updates_use_semantic_versions_and_skip_previews_and_downgrades() {
        assert!(
            newer_version(&release("v0.10.0"), "0.9.0")
                .unwrap()
                .is_some()
        );
        for tag in ["v0.2.2", "v0.2.1", "v0.3.0-beta.1"] {
            assert!(newer_version(&release(tag), "0.2.2").unwrap().is_none());
        }
        let mut draft = release("v1.0.0");
        draft.draft = true;
        assert!(newer_version(&draft, "0.2.2").unwrap().is_none());
    }
    #[test]
    fn assets_must_match_platform_and_stay_in_the_release_repository() {
        assert!(asset_names("1.0.0", "macos", "aarch64")[0].ends_with("universal.zip"));
        assert!(asset_names("1.0.0", "windows", "aarch64").is_empty());
        assert!(
            validate_asset_url("https://evil.test/payload", "owner/repo", "v1.0.0", "a.zip")
                .is_err()
        );
        assert!(
            validate_asset_url(
                "https://github.com/owner/repo/releases/download/v1.0.0/a.zip",
                "owner/repo",
                "v1.0.0",
                "a.zip"
            )
            .is_ok()
        );
        assert!(!is_installed(
            Path::new("/repo/target/release/modo"),
            "linux"
        ));
    }
    #[test]
    fn corrupt_downloads_are_never_installed() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"abc").unwrap();
        let checksum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_checksum(file.path(), checksum).is_ok());
        assert!(verify_checksum(file.path(), &"0".repeat(64)).is_err());
        assert!(verify_checksum(file.path(), "bad").is_err());
        assert_eq!(
            checksum_for(&format!("{checksum}  modo.zip\n"), "modo.zip"),
            Some(checksum.into())
        );
        assert!(checksum_for(&format!("{checksum}  other.zip\n"), "modo.zip").is_none());
    }
}
