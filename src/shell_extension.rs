//! Openhub's GNOME Shell extension (`extension/` in the repository), as
//! GNOME Shell's extension service (`org.gnome.Shell.Extensions`) sees it:
//! whether it's installed, on, or failed, and turning it on and off.

use gtk::glib::prelude::*;
use gtk::{gio, glib};

pub const UUID: &str = "openhub@cjts15.github.io";

/// Installs it from a checkout of the repository; GNOME Shell loads a newly
/// installed extension at the next login.
pub const INSTALL_COMMAND: &str = "gnome-extensions pack extension/openhub@cjts15.github.io --force --extra-source=devices.js --extra-source=icons && gnome-extensions install --force openhub@cjts15.github.io.shell-extension.zip";

const NAME: &str = "org.gnome.Shell.Extensions";
const PATH: &str = "/org/gnome/Shell/Extensions";

/// `ExtensionState.ERROR` in GNOME Shell.
const ERROR: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Not installed, or installed since this login and not loaded yet.
    Missing,
    /// Installed; whether it's turned on.
    Installed { enabled: bool },
    /// It failed to start, with GNOME Shell's message.
    Failed(String),
    /// Every user extension is turned off in GNOME.
    ExtensionsOff,
}

/// The status from `GetExtensionInfo`'s state, enabled flag and error, and
/// whether user extensions are allowed at all.
fn status(info: &glib::VariantDict, user_extensions: bool) -> Status {
    let state: Option<f64> = info.lookup_value("state", None).and_then(|v| v.get());
    let Some(state) = state else {
        return Status::Missing;
    };
    if !user_extensions {
        return Status::ExtensionsOff;
    }
    let enabled = info
        .lookup_value("enabled", None)
        .and_then(|v| v.get())
        .unwrap_or(false);
    let error: String = info
        .lookup_value("error", None)
        .and_then(|v| v.get())
        .unwrap_or_default();
    if state == ERROR {
        Status::Failed(error)
    } else {
        Status::Installed { enabled }
    }
}

/// The extension's status, or `Err` where GNOME Shell isn't running.
pub async fn read(bus: &gio::DBusConnection) -> Result<Status, glib::Error> {
    let reply = bus
        .call_future(
            Some(NAME),
            PATH,
            NAME,
            "GetExtensionInfo",
            Some(&(UUID,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await?;
    let user_extensions = bus
        .call_future(
            Some(NAME),
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(NAME, "UserExtensionsEnabled").to_variant()),
            Some(glib::VariantTy::new("(v)").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await
        .ok()
        .and_then(|v| v.child_value(0).as_variant()?.get())
        .unwrap_or(true);
    let info = glib::VariantDict::new(Some(&reply.child_value(0)));
    Ok(status(&info, user_extensions))
}

pub async fn set_enabled(bus: &gio::DBusConnection, enabled: bool) -> Result<(), glib::Error> {
    let method = if enabled {
        "EnableExtension"
    } else {
        "DisableExtension"
    };
    bus.call_future(
        Some(NAME),
        PATH,
        NAME,
        method,
        Some(&(UUID,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    )
    .await
    .map(drop)
}

/// Calls `changed` when the extension changes state, or user extensions are
/// turned on or off, including from GNOME's Extensions app.
pub fn watch(
    bus: &gio::DBusConnection,
    changed: impl Fn() + 'static,
) -> [gio::SignalSubscription; 2] {
    let changed = std::rc::Rc::new(changed);
    let on_state = changed.clone();
    [
        bus.subscribe_to_signal(
            Some(NAME),
            Some(NAME),
            Some("ExtensionStateChanged"),
            Some(PATH),
            Some(UUID),
            gio::DBusSignalFlags::NONE,
            move |_| on_state(),
        ),
        bus.subscribe_to_signal(
            Some(NAME),
            Some("org.freedesktop.DBus.Properties"),
            Some("PropertiesChanged"),
            Some(PATH),
            Some(NAME),
            gio::DBusSignalFlags::NONE,
            move |_| changed(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(text: &str) -> glib::VariantDict {
        let value =
            glib::Variant::parse(Some(glib::VariantTy::new("a{sv}").unwrap()), text).unwrap();
        glib::VariantDict::new(Some(&value))
    }

    #[test]
    fn reads_the_extension_state() {
        // As GNOME Shell 51 reports it.
        let on = info("{'state': <1.0>, 'enabled': <true>, 'error': <''>}");
        assert_eq!(status(&on, true), Status::Installed { enabled: true });
        let off = info("{'state': <2.0>, 'enabled': <false>, 'error': <''>}");
        assert_eq!(status(&off, true), Status::Installed { enabled: false });
        let failed = info("{'state': <3.0>, 'enabled': <true>, 'error': <'No property vertical'>}");
        assert_eq!(
            status(&failed, true),
            Status::Failed("No property vertical".into())
        );
        assert_eq!(status(&info("@a{sv} {}"), true), Status::Missing);
        assert_eq!(status(&on, false), Status::ExtensionsOff);
    }
}
