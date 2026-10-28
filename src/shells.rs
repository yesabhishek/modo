//! Terminal profiles: the shells installed on this machine, found once, plus
//! the user's own command.
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// The profile that runs the command typed in Settings.
pub const CUSTOM: &str = "Custom";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Shown in menus and as the tab title.
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

impl Profile {
    fn new(name: &str, program: impl AsRef<Path>, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            program: program.as_ref().to_string_lossy().into_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }
}

/// Shells installed on this machine, the platform default first.
pub fn detected() -> &'static [Profile] {
    static PROFILES: OnceLock<Vec<Profile>> = OnceLock::new();
    PROFILES.get_or_init(detect)
}

/// Every profile a user can pick: detected shells, then their custom command.
pub fn available(custom: &str) -> Vec<Profile> {
    let mut profiles = detected().to_vec();
    let words = split_command(custom);
    if let Some((program, args)) = words.split_first() {
        profiles.push(Profile {
            name: CUSTOM.to_owned(),
            program: program.clone(),
            args: args.to_vec(),
        });
    }
    profiles
}

/// The profile called `name`, else the default shell. An empty name with a
/// custom command means the command, as before profiles existed.
pub fn resolve(name: &str, custom: &str) -> Option<Profile> {
    let profiles = available(custom);
    let name = if name.is_empty() && !custom.trim().is_empty() {
        CUSTOM
    } else {
        name
    };
    profiles
        .iter()
        .find(|p| p.name == name)
        .or_else(|| profiles.first())
        .cloned()
}

/// Split a command line into words. Quotes group words with spaces, and
/// backslashes stay literal so Windows paths need no escaping.
pub fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    for c in command.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn detect() -> Vec<Profile> {
    let system32 = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32");
    let program_files = |relative: &str| {
        [
            "ProgramW6432",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "LOCALAPPDATA",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(relative))
        .find(|p| p.is_file())
    };
    let mut profiles = Vec::new();
    // PowerShell 7 starts faster and completes better than Windows PowerShell 5.
    if let Some(pwsh) = find_on_path("pwsh.exe").or_else(|| program_files(r"PowerShell\7\pwsh.exe"))
    {
        profiles.push(Profile::new("PowerShell", pwsh, &["-NoLogo"]));
    }
    let powershell = system32.join(r"WindowsPowerShell\v1.0\powershell.exe");
    if powershell.is_file() {
        profiles.push(Profile::new("Windows PowerShell", powershell, &["-NoLogo"]));
    }
    let cmd = std::env::var_os("ComSpec")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| system32.join("cmd.exe"));
    if cmd.is_file() {
        profiles.push(Profile::new("Command Prompt", cmd, &[]));
    }
    // Git for Windows' launcher, found next to git.exe so custom installs work.
    // (System32\bash.exe is WSL's, listed separately.)
    let git_bash = find_on_path("git.exe")
        .and_then(|git| {
            git.ancestors()
                .skip(1)
                .take(3)
                .map(|dir| dir.join(r"bin\bash.exe"))
                .find(|p| p.is_file())
        })
        .or_else(|| program_files(r"Git\bin\bash.exe"))
        .or_else(|| program_files(r"Programs\Git\bin\bash.exe"));
    if let Some(bash) = git_bash {
        profiles.push(Profile::new("Git Bash", bash, &["--login", "-i"]));
    }
    let wsl = system32.join("wsl.exe");
    if wsl.is_file() {
        profiles.push(Profile::new("WSL", wsl, &[]));
    }
    profiles
}

#[cfg(not(windows))]
fn detect() -> Vec<Profile> {
    // Shells worth offering; anything else appears only as the login shell.
    const KNOWN: [(&str, &str); 5] = [
        ("zsh", "zsh"),
        ("bash", "bash"),
        ("fish", "fish"),
        ("nu", "Nushell"),
        ("pwsh", "PowerShell"),
    ];
    let login = std::env::var("SHELL")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/bash"
            })
        });
    let mut candidates = vec![login.clone()];
    if let Ok(shells) = std::fs::read_to_string("/etc/shells") {
        candidates.extend(
            shells
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with('/'))
                .map(PathBuf::from),
        );
    }
    // Apps opened from the Finder get a minimal PATH, so look where package managers install.
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        candidates.extend(KNOWN.iter().map(|(file, _)| Path::new(dir).join(file)));
    }
    candidates.extend(KNOWN.iter().filter_map(|(file, _)| find_on_path(file)));

    let mut profiles: Vec<Profile> = Vec::new();
    for path in candidates {
        let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let known = KNOWN.iter().find(|(name, _)| *name == file);
        let name = match known {
            Some((_, label)) => *label,
            None if path == login => file,
            None => continue,
        };
        if profiles.iter().any(|p| p.name == name) || !path.is_file() {
            continue;
        }
        // Login shells read the profile files people keep PATH in.
        let args: &[&str] = if file == "pwsh" {
            &["-l", "-NoLogo"]
        } else {
            &["-l"]
        };
        profiles.push(Profile::new(name, &path, args));
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_split_on_spaces_outside_quotes() {
        assert_eq!(
            split_command(r#""C:\Program Files\Git\bin\bash.exe" --login -i"#),
            [r"C:\Program Files\Git\bin\bash.exe", "--login", "-i"]
        );
        assert_eq!(
            split_command("  /bin/bash   --noprofile "),
            ["/bin/bash", "--noprofile"]
        );
        assert_eq!(
            split_command("fish -C 'echo hi'"),
            ["fish", "-C", "echo hi"]
        );
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn custom_commands_become_a_profile_and_old_settings_still_use_them() {
        let custom = resolve("", "/bin/bash --noprofile").unwrap();
        assert_eq!(custom.name, CUSTOM);
        assert_eq!(custom.program, "/bin/bash");
        assert_eq!(custom.args, ["--noprofile"]);
        assert_eq!(resolve(CUSTOM, "nu").unwrap().program, "nu");
        // Without a custom command, "Custom" falls back to the default shell.
        assert_eq!(resolve(CUSTOM, "").as_ref(), detected().first());
    }

    #[test]
    fn this_machine_has_a_default_shell() {
        let default = resolve("", "").expect("a shell");
        assert!(Path::new(&default.program).is_file());
        let names: Vec<_> = detected().iter().map(|p| &p.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len());
    }
}
