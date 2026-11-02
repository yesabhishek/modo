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
