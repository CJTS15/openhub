pub mod audio;
pub mod bluetooth;
pub mod display;
pub mod input;
pub mod power;
pub mod system;
pub mod usb;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

/// Runs `rescan` when `widget` is shown and every 2 seconds while it stays
/// shown. sysfs has no change signal Openhub can watch from the sandbox,
/// and a hidden page costs nothing this way.
pub fn rescan_while_mapped(widget: &impl IsA<gtk::Widget>, rescan: impl Fn() + 'static) {
    let rescan = Rc::new(rescan);
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let start = Rc::new(glib::clone!(
        #[strong]
        timer,
        move || {
            rescan();
            let rescan = rescan.clone();
            let source = glib::timeout_add_seconds_local(2, move || {
                rescan();
                glib::ControlFlow::Continue
            });
            if let Some(old) = timer.replace(Some(source)) {
                old.remove();
            }
        }
    ));
    widget.connect_map(glib::clone!(
        #[strong]
        start,
        move |_| start()
    ));
    widget.connect_unmap(move |_| {
        if let Some(source) = timer.take() {
            source.remove();
        }
    });
    // A widget added to a page already on screen never gets "map".
    if widget.is_mapped() {
        start();
    }
}

/// Opens a GNOME Settings panel, such as `keyboard`, or a page inside one,
/// such as `system/about`.
pub fn open_settings_panel(widget: &impl IsA<gtk::Widget>, panel: &'static str) {
    // Wayland only lets GNOME Settings come to the front with a token from
    // the app that asked.
    let token = widget
        .display()
        .app_launch_context()
        .startup_notify_id(None::<&gio::AppInfo>, &[]);
    glib::spawn_future_local(async move {
        if let Err(error) = launch_panel(panel, token.as_deref()).await {
            glib::g_warning!("openhub", "Can't open GNOME Settings: {error}");
        }
    });
}

/// GNOME Settings' `launch-panel` action.
async fn launch_panel(panel: &str, token: Option<&str>) -> Result<(), glib::Error> {
    let bus = gio::bus_get_future(gio::BusType::Session).await?;
    let mut parts = panel.split('/');
    let name = parts.next().unwrap_or(panel);
    let pages: Vec<glib::Variant> = parts.map(|page| page.to_variant()).collect();
    let panel = (name, pages).to_variant();
    let args = (
        "launch-panel",
        vec![panel],
        token
            .map(|token| {
                HashMap::from([
                    ("activation-token".to_owned(), token.to_variant()),
                    ("desktop-startup-id".to_owned(), token.to_variant()),
                ])
            })
            .unwrap_or_default(),
    )
        .to_variant();
    bus.call_future(
        Some("org.gnome.Settings"),
        "/org/gnome/Settings",
        "org.gtk.Actions",
        "Activate",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    )
    .await
    .map(drop)
}
