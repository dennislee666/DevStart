//! Public application identity. Release surfaces derive their version from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHOR: &str = "dennis lee";
pub const REPOSITORY: &str = "https://github.com/dennislee666/DevStart";
pub const CONTACT: &str = "danislee@gmail.com";
pub const CONTACT_URL: &str = "mailto:danislee@gmail.com";

/// Hide a zero patch component in the human-facing version (1.0.0 → 1.0).
pub fn display_version() -> &'static str {
    VERSION.strip_suffix(".0").unwrap_or(VERSION)
}
