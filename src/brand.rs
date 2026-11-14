//! The application's identity, kept in one place so a rename touches one file.

pub const NAME: &str = "Modo";
/// Executable, CLI, and configuration directory name.
pub const SLUG: &str = "modo";
pub const BUNDLE_ID: &str = "dev.modo.Modo";
pub const TAGLINE: &str = "Read, review, and edit what your agents wrote.";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Publisher identity used by the About surfaces and release metadata.
pub const PUBLISHER: &str = "Abhishek Choudhury";
pub const PUBLISHER_URL: &str = env!("CARGO_PKG_HOMEPAGE");

/// Ed25519 public key (hex) that signs each release's SHA256SUMS file.
/// `scripts/release-key.sh` creates the pair; the private half lives only in
/// the `RELEASE_SIGNING_KEY` repository secret. While this is empty, Modo
/// reports new releases but never installs one by itself.
pub const RELEASE_KEY: &str = "1d8ed895e492ca09d7b1c208791423af0da639746eac66e9284ea4c3719defbc";
