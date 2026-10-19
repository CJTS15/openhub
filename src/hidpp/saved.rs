//! Each mouse's saved settings. Mice forget them when switched off (tested
//! on an MX Master 3S), so udev has the user's systemd run
//! `openhub --apply-mice hidrawN` when one connects (see
//! `data/udev/70-openhub-logitech.rules`), which sends them again and exits.
//! Nothing runs in between.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::glib;

use super::{Settings, Wheel};

/// The mouse's saved settings, by `Node::uniq`.
fn saved(uniq: &str) -> Option<Settings> {
    load().into_iter().find(|(u, _)| u == uniq).map(|(_, s)| s)
}

/// Remembers a mouse's settings, so they're sent again when it reconnects.
pub fn save(uniq: &str, settings: Settings) -> Result<(), String> {
    let mut saved = load();
    match saved.iter_mut().find(|(u, _)| u == uniq) {
        Some((_, old)) => *old = settings,
        None => saved.push((uniq.to_owned(), settings)),
    }
    store(&saved)
}

/// Sends the saved settings to the mouse at `/dev/<node>`, if it's a
/// Logitech mouse with any. Blocks; it's run by `openhub --apply-mice`.
pub fn apply(node: &str) -> Result<(), String> {
    let path = Path::new("/dev").join(node);
    let Some(mouse) = super::find().into_iter().find(|n| n.path == path) else {
        return Ok(());
    };
    let Some(settings) = saved(&mouse.uniq) else {
        return Ok(());
    };
    // A mouse just switched on can take a moment to answer.
    let mut result = Ok(());
    for _ in 0..3 {
        result = super::write(&path, &settings).map_err(|e| e.to_string());
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    result
}

fn path() -> PathBuf {
    glib::user_config_dir().join("openhub/mice.ini")
}

fn load() -> Vec<(String, Settings)> {
    let file = glib::KeyFile::new();
    if file
        .load_from_file(path(), glib::KeyFileFlags::NONE)
        .is_err()
    {
        return Vec::new();
    }
    from_keyfile(&file)
}

fn from_keyfile(file: &glib::KeyFile) -> Vec<(String, Settings)> {
    file.groups()
        .iter()
        .map(|uniq| {
            let uniq = uniq.as_str();
            let dpi = file
                .integer(uniq, "dpi")
                .ok()
                .and_then(|dpi| u16::try_from(dpi).ok());
            let wheel = file
                .string(uniq, "wheel")
                .ok()
                .and_then(|wheel| match wheel.as_str() {
                    "ratchet" => Some(Wheel::Ratchet),
                    "free-spin" => Some(Wheel::FreeSpin),
                    other => Some(Wheel::SmartShift(
                        other.strip_prefix("smartshift:")?.parse().ok()?,
                    )),
                });
            let inverted = file.boolean(uniq, "inverted").ok();
            (
                uniq.to_owned(),
                Settings {
                    dpi,
                    wheel,
                    inverted,
                },
            )
        })
        .collect()
}

fn store(saved: &[(String, Settings)]) -> Result<(), String> {
    let file = to_keyfile(saved);
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not save mouse settings: {e}"))?;
    }
    glib::file_set_contents(&path, file.to_data().as_bytes())
        .map_err(|e| format!("Could not save mouse settings: {e}"))
}

fn to_keyfile(saved: &[(String, Settings)]) -> glib::KeyFile {
    let file = glib::KeyFile::new();
    for (uniq, settings) in saved {
        if let Some(dpi) = settings.dpi {
            file.set_integer(uniq, "dpi", dpi.into());
        }
        if let Some(wheel) = settings.wheel {
            let wheel = match wheel {
                Wheel::SmartShift(threshold) => format!("smartshift:{threshold}"),
                Wheel::Ratchet => "ratchet".to_owned(),
                Wheel::FreeSpin => "free-spin".to_owned(),
            };
            file.set_string(uniq, "wheel", &wheel);
        }
        if let Some(inverted) = settings.inverted {
            file.set_boolean(uniq, "inverted", inverted);
        }
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_the_key_file() {
        let saved = vec![(
            "0a:00:00:00:00:01".to_owned(),
            Settings {
                dpi: Some(1600),
                wheel: Some(Wheel::SmartShift(12)),
                inverted: Some(false),
            },
        )];
        let text = to_keyfile(&saved).to_data();
        let file = glib::KeyFile::new();
        file.load_from_data(&text, glib::KeyFileFlags::NONE)
            .unwrap();
        let loaded = from_keyfile(&file);
        assert_eq!(loaded, saved);
    }
}
