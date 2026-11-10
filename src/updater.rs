//! Release updates: a check at startup, a manual check from Settings > About,
//! and noticing when the app on disk is no longer the one that is running.
//! Only packaged installs replace themselves; source builds and capture runs
//! never touch a developer's binary.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
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

/// Where an update check stands. Settings > About and the status bar read it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A newer release exists. `installable` is false for source builds, which
    /// are pointed at the release page instead of replacing themselves.
    Available {
        version: String,
        url: String,
        installable: bool,
    },
    Downloading {
        version: String,
        percent: Option<u8>,
    },
    /// The new version is on disk; this process is still the old one. The
    /// version is unknown when something other than the updater replaced the app.
    Ready {
        version: Option<String>,
    },
    /// Downloaded, but the operating system's installer has to finish.
    Package {
        version: String,
        path: PathBuf,
    },
    Failed(String),
}

#[derive(Clone, Default)]
pub struct UpdateState {
    pub phase: Phase,
}
impl Global for UpdateState {}

impl UpdateState {
    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Checking | Phase::Downloading { .. })
    }

    /// One line for Settings > About.
    pub fn status(&self) -> SharedString {
        match &self.phase {
            Phase::Idle => "Check whether a newer release is out.".into(),
            Phase::Checking => "Checking for updates…".into(),
            Phase::UpToDate => "You have the latest version.".into(),
            Phase::Available { version, .. } => format!("Modo {version} is available.").into(),
            Phase::Downloading { version, percent } => match percent {
                Some(percent) => format!("Downloading Modo {version}… {percent}%").into(),
                None => format!("Downloading Modo {version}…").into(),
            },
            Phase::Ready { version } => match version {
                Some(version) if cfg!(target_os = "windows") => {
                    format!("Modo {version} installs when you quit.").into()
                }
                Some(version) => format!("Modo {version} is installed. Restart to use it.").into(),
                None => "A newer Modo was installed. Restart to use it.".into(),
            },
            Phase::Package { version, .. } => {
                format!("Modo {version} is downloaded and ready to install.").into()
            }
            Phase::Failed(error) => format!("Could not check for updates: {error}").into(),
        }
    }

    /// What the status bar shows: only the states that need the user.
    pub fn notice(&self) -> Option<SharedString> {
        match &self.phase {
            Phase::Downloading { .. } => Some(self.status()),
            Phase::Available { version, .. } => Some(format!("Modo {version} available").into()),
            Phase::Ready { .. } => Some("Restart to update".into()),
            Phase::Package { version, .. } => Some(format!("Install Modo {version}").into()),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
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
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// The executable as it was at launch, to notice when it is replaced under us.
#[derive(Clone, PartialEq)]
struct Identity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    inode: u64,
}

static LAUNCHED: OnceLock<Option<(PathBuf, Identity)>> = OnceLock::new();
static RESTART: AtomicBool = AtomicBool::new(false);

fn identity(path: &Path) -> Option<Identity> {
    let metadata = fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&metadata);
    #[cfg(not(unix))]
    let inode = 0;
    Some(Identity {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        inode,
    })
}

fn launched() -> Option<&'static (PathBuf, Identity)> {
    LAUNCHED
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let identity = identity(&exe)?;
            Some((exe, identity))
        })
        .as_ref()
}

fn dormant() -> bool {
    std::env::var_os("MODO_TEMPORARY_SESSION").is_some()
        || std::env::args_os().any(|arg| arg == "--capture")
}

/// Only packaged release builds replace themselves.
fn self_updating(exe: &Path) -> bool {
    !cfg!(debug_assertions) && is_installed(exe, std::env::consts::OS)
}

pub fn init(cx: &mut App) {
    cx.set_global(UpdateState::default());
    if dormant() {
        return;
    }
    let installed = launched().is_some_and(|(exe, _)| self_updating(exe));
    if !installed || !Settings::get(cx).auto_update {
        return;
    }
    cx.spawn(async move |cx| {
        // Let the first frame and session restoration finish first.
        cx.background_executor().timer(Duration::from_secs(3)).await;
        cx.update(|cx| {
            if Settings::get(cx).auto_update {
                start(Mode::Automatic, cx);
            }
        });
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// At startup: install quietly, and keep failures out of the way.
    Automatic,
    /// The Check for updates button: report what is out, install nothing yet.
    Check,
    /// The Download button.
    Install,
}

/// Look for a newer release now, whatever the automatic setting says.
pub fn check(cx: &mut App) {
    start(Mode::Check, cx);
}

/// Download and install the release a check found.
pub fn install_available(cx: &mut App) {
    match UpdateState::get(cx).phase.clone() {
        Phase::Available {
            installable: true, ..
        } => start(Mode::Install, cx),
        Phase::Available { url, .. } => cx.open_url(&url),
        Phase::Package { path, .. } => {
            let _ = open::that(path);
        }
        _ => {}
    }
}

fn start(mode: Mode, cx: &mut App) {
    let state = UpdateState::get(cx);
    if state.busy() || matches!(state.phase, Phase::Ready { .. } | Phase::Package { .. }) {
        return;
    }
    let exe = launched().map(|(exe, _)| exe.clone());
    let installable = !dormant() && exe.as_deref().is_some_and(self_updating);
    let previous = state.phase.clone();
    cx.set_global(UpdateState {
        phase: Phase::Checking,
    });
    let (tx, mut rx) = mpsc::unbounded::<Phase>();
    cx.spawn(async move |cx| {
        let worker = cx.background_executor().spawn(async move {
            let install = installable && mode != Mode::Check;
            let result = match &exe {
                Some(exe) => check_and_install(exe, install, installable, &tx),
                None => Err("Modo cannot find its own executable".into()),
            };
            let last = match result {
                Ok(phase) => phase,
                Err(error) => {
                    crate::startup::log(format!("update: {error}"));
                    if mode == Mode::Automatic {
                        // Being offline at launch is not worth a message.
                        previous
                    } else {
                        Phase::Failed(error)
                    }
                }
            };
            let _ = tx.unbounded_send(last);
        });
        while let Some(phase) = rx.next().await {
            cx.update(|cx| cx.set_global(UpdateState { phase }));
        }
        worker.await;
    })
    .detach();
}

/// The app on disk is no longer the one that is running: an update, an
/// installer, or `scripts/install.sh` replaced it while Modo was open.
pub fn check_replaced(cx: &mut App) {
    if dormant() || cfg!(debug_assertions) {
        return;
    }
    let state = UpdateState::get(cx);
    if state.busy() || matches!(state.phase, Phase::Ready { .. }) {
        return;
    }
    let Some((exe, original)) = launched() else {
        return;
    };
    cx.spawn(async move |cx| {
        let replaced = cx
            .background_executor()
            .spawn(async move { was_replaced(exe, original) })
            .await;
        if replaced {
            cx.update(|cx| {
                if !UpdateState::get(cx).busy() {
                    cx.set_global(UpdateState {
                        phase: Phase::Ready { version: None },
                    });
                }
            });
        }
    })
    .detach();
}

/// A missing file is an install in progress, not a finished one.
fn was_replaced(exe: &Path, original: &Identity) -> bool {
    identity(exe).is_some_and(|current| &current != original)
}

/// Quit and come back as the installed version. Every window still asks about
/// unsaved files first; cancelling one cancels the restart.
pub fn restart(cx: &mut App) {
    RESTART.store(true, Ordering::Relaxed);
    crate::ui::workspace::quit(cx);
}

pub fn cancel_restart() {
    RESTART.store(false, Ordering::Relaxed);
}

/// Called once every window agreed to close.
pub fn finish_quit(cx: &mut App) {
    if !RESTART.swap(false, Ordering::Relaxed) {
        cx.quit();
        return;
    }
    if cfg!(target_os = "windows") {
        // The waiting installer starts the new version once it has finished.
        if let Some(dir) = settings::config_dir() {
            let _ = fs::write(dir.join("updates").join("relaunch"), b"");
        }
        cx.quit();
        return;
    }
    // macOS reopens the bundle at its path. Elsewhere, name the path the binary
    // was launched from: the running image may already be an unlinked file.
    if !cfg!(target_os = "macos")
        && let Some((exe, _)) = launched()
    {
        cx.set_restart_path(exe.clone());
    }
    cx.restart();
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

/// Returns the phase the check ends in. `install` downloads and installs a
/// newer release; without it the release is only reported.
fn check_and_install(
    exe: &Path,
    wanted: bool,
    installable: bool,
    tx: &mpsc::UnboundedSender<Phase>,
) -> Result<Phase, String> {
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
        return Err("Another Modo window is already checking".into());
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
        None,
    )?;
    let release: Release =
        serde_json::from_slice(&fs::read(metadata_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let Some(version) = newer_version(&release, brand::VERSION)? else {
        return Ok(Phase::UpToDate);
    };
    let version = version.to_string();
    let page = if release.html_url.starts_with("https://github.com/") {
        release.html_url.clone()
    } else {
        format!("{}/releases", brand::REPOSITORY)
    };
    if !wanted {
        return Ok(Phase::Available {
            version,
            url: page,
            installable,
        });
    }
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
                None,
            )?;
            checksum_for(
                &fs::read_to_string(checksum_path).map_err(|e| e.to_string())?,
                &asset.name,
            )
            .ok_or("The package is missing from SHA256SUMS")?
        }
    };
    let _ = tx.unbounded_send(Phase::Downloading {
        version: version.clone(),
        percent: None,
    });
    let package = stage.path().join(&asset.name);
    let progress = |percent: u8| {
        let _ = tx.unbounded_send(Phase::Downloading {
            version: version.clone(),
            percent: Some(percent),
        });
    };
    download(
        &asset.browser_download_url,
        &package,
        512 * 1024 * 1024,
        300,
        Some((asset.size, &progress)),
    )?;
    verify_checksum(&package, &expected)?;
    if let Err(error) = install(exe, &package, &version, &dir) {
        crate::startup::log(format!("update installation: {error}"));
        if package.extension().is_some_and(|ext| ext == "deb") {
            // A system package: hand it to the package installer.
            let path = package.clone();
            let _ = stage.keep();
            return Ok(Phase::Package { version, path });
        }
        return Err(error);
    }
    #[cfg(target_os = "windows")]
    {
        // The installer and its waiting helper must outlive this process.
        let _ = stage.keep();
    }
    Ok(Phase::Ready {
        version: Some(version),
    })
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

type Progress<'a> = (u64, &'a dyn Fn(u8));

/// Fetch `url` with curl. With `progress`, the expected size and a callback,
/// the growing file is sampled so the interface can show how far along it is.
fn download(
    url: &str,
    target: &Path,
    limit: u64,
    seconds: u64,
    progress: Option<Progress<'_>>,
) -> Result<(), String> {
    let mut child = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot download update: {e}"))?;
    let mut last = None;
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(120)),
        }
        if let Some((total, report)) = progress
            && total > 0
        {
            let done = fs::metadata(target).map(|m| m.len()).unwrap_or(0);
            let percent = (done.saturating_mul(100) / total).min(99) as u8;
            if last != Some(percent) {
                last = Some(percent);
                report(percent);
            }
        }
    };
    if !status.success() {
        let mut message = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut message);
        }
        let message: String = message.trim().chars().take(200).collect();
        return Err(if message.is_empty() {
            "The download failed".into()
        } else {
            message
        });
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
fn install(exe: &Path, package: &Path, _version: &str, updates: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let dir = exe.parent().ok_or("No installation folder")?;
    let script = package
        .parent()
        .ok_or("No update staging folder")?
        .join("install.ps1");
    let relaunch = updates.join("relaunch");
    let _ = fs::remove_file(&relaunch);
    fs::write(&script, r#"param([int]$ParentId, [string]$Installer, [string]$InstallDir, [string]$Relaunch)
$ErrorActionPreference = 'Stop'
Wait-Process -Id $ParentId -ErrorAction SilentlyContinue
while (Get-Process -Name modo -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $InstallDir 'modo.exe') }) { Start-Sleep -Seconds 1 }
$process = Start-Process -FilePath $Installer -ArgumentList @('/S', ('/D=' + $InstallDir)) -Wait -PassThru
if ($process.ExitCode -ne 0) { throw ('Modo installer failed: ' + $process.ExitCode) }
Remove-Item -LiteralPath $Installer -Force
# "Restart to update" leaves this marker: come back as the new version.
if (Test-Path -LiteralPath $Relaunch) {
    Remove-Item -LiteralPath $Relaunch -Force
    Start-Process -FilePath (Join-Path $InstallDir 'modo.exe')
}
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
        .arg("-Relaunch")
        .arg(&relaunch)
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
            html_url: String::new(),
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
    fn a_replaced_executable_is_noticed_but_a_missing_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("modo");
        fs::write(&exe, b"old build").unwrap();
        let original = identity(&exe).unwrap();
        assert!(!was_replaced(&exe, &original));
        // Mid-install the file is gone for a moment; that is not an update yet.
        fs::remove_file(&exe).unwrap();
        assert!(!was_replaced(&exe, &original));
        fs::write(&exe, b"a newer and longer build").unwrap();
        assert!(was_replaced(&exe, &original));
    }

    #[test]
    fn the_status_bar_only_speaks_when_the_user_is_needed() {
        let state = |phase| UpdateState { phase };
        for quiet in [
            Phase::Idle,
            Phase::Checking,
            Phase::UpToDate,
            Phase::Failed("offline".into()),
        ] {
            assert!(state(quiet).notice().is_none());
        }
        let ready = state(Phase::Ready {
            version: Some("9.9.9".into()),
        });
        assert_eq!(ready.notice().unwrap().as_ref(), "Restart to update");
        assert!(ready.status().contains("9.9.9"));
        let downloading = state(Phase::Downloading {
            version: "9.9.9".into(),
            percent: Some(42),
        });
        assert!(downloading.busy());
        assert!(downloading.notice().unwrap().contains("42%"));
        assert!(
            state(Phase::Failed("offline".into()))
                .status()
                .contains("offline")
        );
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
