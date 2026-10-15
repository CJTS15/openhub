//! Keeps Logitech mice on their saved settings. Mice forget them when turned
//! off (tested on an MX Master 3S), so the keeper sends them again whenever
//! a mouse's hidraw node reappears in `/dev`, which needs no polling.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::{Settings, Wheel};

pub struct Keeper {
    /// Saved settings by mouse (`Node::uniq`).
    saved: RefCell<Vec<(String, Settings)>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl Keeper {
    pub fn shared() -> Rc<Keeper> {
        thread_local! {
            static SHARED: Rc<Keeper> = Rc::new(Keeper {
                saved: RefCell::new(load()),
                monitor: RefCell::default(),
                hold: RefCell::default(),
            });
        }
        SHARED.with(Clone::clone)
    }

    /// Applies saved settings to the mice present now and watches for mice
    /// coming back. Keeps `app` running while there's anything to keep.
    pub fn start(self: &Rc<Self>, app: &impl IsA<gio::Application>) {
        self.update_hold(app.upcast_ref());
        let keeper = Rc::downgrade(self);
        let monitor = gio::File::for_path("/dev")
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE);
        if let Ok(monitor) = monitor {
            monitor.connect_changed(move |_, file, _, event| {
                let new = event == gio::FileMonitorEvent::Created;
                let name = file.basename().unwrap_or_default();
                if let (true, Some(keeper)) = (new, keeper.upgrade())
                    && name.to_string_lossy().starts_with("hidraw")
                {
                    keeper.restore(file.path());
                }
            });
            self.monitor.replace(Some(monitor));
        }
        for node in super::find() {
            self.restore(Some(node.path));
        }
    }

    pub fn saved(&self, uniq: &str) -> Option<Settings> {
        let saved = self.saved.borrow();
        saved.iter().find(|(u, _)| u == uniq).map(|(_, s)| *s)
    }

    /// Remembers a mouse's settings, so they're sent again after it reconnects.
    pub fn save(
        &self,
        app: &gio::Application,
        uniq: &str,
        settings: Settings,
    ) -> Result<(), String> {
        let first = self.saved.borrow().is_empty();
        {
            let mut saved = self.saved.borrow_mut();
            match saved.iter_mut().find(|(u, _)| u == uniq) {
                Some((_, old)) => *old = settings,
                None => saved.push((uniq.to_owned(), settings)),
            }
        }
        self.update_hold(app);
        if first {
            request_background();
        }
        store(&self.saved.borrow())
    }

    fn update_hold(&self, app: &gio::Application) {
        let wanted = !self.saved.borrow().is_empty();
        if wanted != self.hold.borrow().is_some() {
            self.hold.replace(wanted.then(|| app.hold()));
        }
    }

    /// Sends a mouse's saved settings once its node at `path` answers. A
    /// node just created may still lack the user's access, and a mouse just
    /// switched on takes a moment, so it tries a few times.
    fn restore(self: &Rc<Self>, path: Option<PathBuf>) {
        let Some(path) = path else {
            return;
        };
        let keeper = self.clone();
        glib::spawn_future_local(async move {
            for delay in [1, 2, 4] {
                glib::timeout_future(Duration::from_secs(delay)).await;
                let Some(node) = super::find().into_iter().find(|node| node.path == path) else {
                    return; // Not a Logitech mouse, or gone again.
                };
                let Some(settings) = keeper.saved(&node.uniq) else {
                    return;
                };
                let write = gio::spawn_blocking(move || super::write(&node.path, &settings));
                if let Ok(Ok(())) = write.await {
                    return;
                }
            }
        });
    }
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

/// Asks the Background portal to let Openhub run without a window and start
/// at login, so mice keep their settings. The answer only changes whether
/// GNOME lists Openhub as a background app, so it isn't awaited.
fn request_background() {
    glib::spawn_future_local(async {
        let result: zbus::Result<()> = async {
            let connection = zbus::Connection::session().await?;
            let mut options = std::collections::HashMap::<&str, zbus::zvariant::Value>::new();
            options.insert(
                "reason",
                "Keep your mouse settings after it reconnects".into(),
            );
            options.insert("autostart", true.into());
            options.insert("commandline", vec!["openhub", "--background"].into());
            connection
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.Background"),
                    "RequestBackground",
                    &("", options),
                )
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            glib::g_warning!("openhub", "Can't ask to run in the background: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_the_key_file() {
        let saved = vec![(
            "d5:79:24:d9:f8:be".to_owned(),
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
