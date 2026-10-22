//! Colour profiles through colord (`org.freedesktop.ColorManager` on the
//! system bus), the service GNOME Settings › Color uses. A monitor is the
//! colord device whose `XRANDR_name` is its connector; its first profile is
//! the one in use.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::glib::prelude::*;
use gtk::{gio, glib};

const NAME: &str = "org.freedesktop.ColorManager";
const PATH: &str = "/org/freedesktop/ColorManager";
const DEVICE: &str = "org.freedesktop.ColorManager.Device";
const PROFILE: &str = "org.freedesktop.ColorManager.Profile";

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub path: String,
    /// The one in use first.
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub path: String,
    pub title: String,
    /// Generated from the monitor's EDID: removing it would leave nothing.
    pub from_edid: bool,
}

pub async fn bus() -> Result<gio::DBusConnection, glib::Error> {
    gio::bus_get_future(gio::BusType::System).await
}

async fn call(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    method: &str,
    args: Option<glib::Variant>,
) -> Result<glib::Variant, glib::Error> {
    bus.call_future(
        Some(NAME),
        path,
        interface,
        method,
        args.as_ref(),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    )
    .await
}

async fn property(
    bus: &gio::DBusConnection,
    path: &str,
    interface: &str,
    name: &str,
) -> Option<glib::Variant> {
    let reply = call(
        bus,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some((interface, name).to_variant()),
    )
    .await
    .ok()?;
    reply.child_value(0).as_variant()
}

fn object_path(path: &str) -> Result<glib::variant::ObjectPath, glib::Error> {
    glib::variant::ObjectPath::try_from(path.to_owned())
        .map_err(|_| glib::Error::new(gio::IOErrorEnum::InvalidArgument, "Not a D-Bus path"))
}

/// The colord device for `connector`, such as `DP-2`, with its profiles.
pub async fn device(bus: &gio::DBusConnection, connector: &str) -> Option<Device> {
    let reply = call(
        bus,
        PATH,
        NAME,
        "FindDeviceByProperty",
        Some(("XRANDR_name", connector).to_variant()),
    )
    .await
    .ok()?;
    let path: String = reply.child_value(0).str()?.to_owned();
    let list = property(bus, &path, DEVICE, "Profiles").await?;
    let mut profiles = Vec::new();
    for i in 0..list.n_children() {
        let profile = list.child_value(i).str()?.to_owned();
        let title = property(bus, &profile, PROFILE, "Title")
            .await
            .and_then(|v| v.get::<String>())
            .unwrap_or_default();
        let metadata = property(bus, &profile, PROFILE, "Metadata")
            .await
            .and_then(|v| v.get::<std::collections::HashMap<String, String>>())
            .unwrap_or_default();
        profiles.push(Profile {
            from_edid: metadata.get("DATA_source").map(String::as_str) == Some("edid"),
            title,
            path: profile,
        });
    }
    Some(Device { path, profiles })
}

pub async fn make_default(
    bus: &gio::DBusConnection,
    device: &str,
    profile: &str,
) -> Result<(), glib::Error> {
    let profile = object_path(profile)?;
    call(
        bus,
        device,
        DEVICE,
        "MakeProfileDefault",
        Some((profile,).to_variant()),
    )
    .await
    .map(drop)
}

/// Takes the profile off the monitor; the file stays.
pub async fn remove(
    bus: &gio::DBusConnection,
    device: &str,
    profile: &str,
) -> Result<(), glib::Error> {
    let profile = object_path(profile)?;
    call(
        bus,
        device,
        DEVICE,
        "RemoveProfile",
        Some((profile,).to_variant()),
    )
    .await
    .map(drop)
}

/// Copies an ICC file into the user's profile store (`~/.local/share/icc`),
/// which GNOME registers with colord, then makes it the monitor's profile.
pub async fn import(
    bus: &gio::DBusConnection,
    device: &str,
    source: PathBuf,
) -> Result<(), glib::Error> {
    // ponytail: in Flatpak this assumes the host keeps the default
    // XDG_DATA_HOME (~/.local/share), which is what xdg-data/icc maps to.
    let store = if std::path::Path::new("/.flatpak-info").exists() {
        glib::home_dir().join(".local/share/icc")
    } else {
        glib::user_data_dir().join("icc")
    };
    let copied = gio::spawn_blocking(move || copy_profile(&source, &store))
        .await
        .map_err(|_| glib::Error::new(gio::IOErrorEnum::Failed, "The import stopped"))?
        .map_err(|message| glib::Error::new(gio::IOErrorEnum::InvalidData, &message))?;
    let filename = copied.to_string_lossy().into_owned();
    // Registration happens in the background, within a second here.
    for _ in 0..25 {
        if let Ok(reply) = call(
            bus,
            PATH,
            NAME,
            "FindProfileByFilename",
            Some((filename.as_str(),).to_variant()),
        )
        .await
        {
            let profile = reply.child_value(0).str().unwrap_or_default().to_owned();
            let path = object_path(&profile)?;
            // "hard": colord keeps it on the monitor across restarts.
            call(
                bus,
                device,
                DEVICE,
                "AddProfile",
                Some(("hard", path).to_variant()),
            )
            .await?;
            return make_default(bus, device, &profile).await;
        }
        glib::timeout_future(Duration::from_millis(200)).await;
    }
    Err(glib::Error::new(
        gio::IOErrorEnum::TimedOut,
        "The color service didn't pick up the profile",
    ))
}

/// Checks the file is a display ICC profile and copies it into `store`,
/// never over another file. Returns where it went.
fn copy_profile(source: &Path, store: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    check_display_profile(&bytes)?;
    std::fs::create_dir_all(store).map_err(|e| e.to_string())?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("profile");
    for n in 0..100 {
        let name = match n {
            0 => format!("{stem}.icc"),
            n => format!("{stem}-{n}.icc"),
        };
        let target = store.join(name);
        match std::fs::read(&target) {
            // Already imported.
            Ok(existing) if existing == bytes => return Ok(target),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
                return Ok(target);
            }
        }
    }
    Err("Too many profiles with this name".into())
}

/// An ICC header: its size first, `acsp` at byte 36, and the display
/// class (`mntr`) at byte 12.
fn check_display_profile(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 128 || &bytes[36..40] != b"acsp" {
        return Err("This isn't an ICC profile".into());
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if size != bytes.len() {
        return Err("This ICC profile is damaged".into());
    }
    if &bytes[12..16] != b"mntr" {
        return Err("This ICC profile isn't for a display".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(class: &[u8; 4], len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        bytes[..4].copy_from_slice(&(len as u32).to_be_bytes());
        bytes[12..16].copy_from_slice(class);
        bytes[36..40].copy_from_slice(b"acsp");
        bytes
    }

    #[test]
    fn accepts_only_display_profiles() {
        assert!(check_display_profile(&header(b"mntr", 200)).is_ok());
        assert!(check_display_profile(&header(b"prtr", 200)).is_err());
        assert!(check_display_profile(b"not a profile").is_err());
        let mut short = header(b"mntr", 200);
        short.truncate(150);
        assert!(check_display_profile(&short).is_err());
    }

    #[test]
    fn copies_without_overwriting() {
        let root = std::env::temp_dir().join(format!("openhub-icc-{}", std::process::id()));
        let (source, store) = (root.join("Panel.icc"), root.join("icc"));
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(&source, header(b"mntr", 200)).unwrap();
        std::fs::write(store.join("Panel.icc"), header(b"mntr", 300)).unwrap();
        let first = copy_profile(&source, &store);
        let again = copy_profile(&source, &store);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(first, Ok(store.join("Panel-1.icc")));
        assert_eq!(again, Ok(store.join("Panel-1.icc")));
    }
}
