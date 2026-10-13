//! Build-time configuration.
//!
//! Meson passes these through the environment when it invokes Cargo, so the
//! values match what it installs. A plain `cargo build` falls back to the
//! development defaults below.

pub const APP_ID: &str = match option_env!("OPENHUB_APP_ID") {
    Some(id) => id,
    None => "io.github.cjts15.Openhub",
};

pub const VERSION: &str = match option_env!("OPENHUB_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// GResource base path. Fixed rather than derived from `APP_ID`, because the
/// `.ui` files reference it literally.
pub const RESOURCE_BASE_PATH: &str = "/io/github/cjts15/Openhub/";
