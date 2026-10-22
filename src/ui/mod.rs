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
    let mut parts = panel.split('/');
    let name = parts.next().unwrap_or(panel);
    let pages: Vec<glib::Variant> = parts.map(|page| page.to_variant()).collect();
    activate_app(
        widget,
        "org.gnome.Settings",
        "launch-panel",
        (name, pages).to_variant(),
    );
}

/// Runs an action of a D-Bus activatable app (`org.gtk.Actions`), such as
/// GNOME Settings' `launch-panel` or GNOME Software's `set-mode`.
pub fn activate_app(
    widget: &impl IsA<gtk::Widget>,
    app: &'static str,
    action: &'static str,
    parameter: glib::Variant,
) {
    // Wayland only lets the app come to the front with a token from the
    // app that asked.
    let token = widget
        .display()
        .app_launch_context()
        .startup_notify_id(None::<&gio::AppInfo>, &[]);
    glib::spawn_future_local(async move {
        if let Err(error) = activate(app, action, parameter, token.as_deref()).await {
            glib::g_warning!("openhub", "Can't open {app}: {error}");
        }
    });
}

async fn activate(
    app: &str,
    action: &str,
    parameter: glib::Variant,
    token: Option<&str>,
) -> Result<(), glib::Error> {
    let bus = gio::bus_get_future(gio::BusType::Session).await?;
    let args = (
        action,
        vec![parameter],
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
    // org.gnome.Settings lives at /org/gnome/Settings.
    let path = format!("/{}", app.replace('.', "/"));
    bus.call_future(
        Some(app),
        &path,
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

/// Whether a D-Bus activatable app, such as `org.gnome.Software`, is
/// installed (or running).
pub async fn app_installed(app: &str) -> bool {
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        return false;
    };
    let call = |method: &'static str| {
        bus.call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            None,
            Some(glib::VariantTy::new("(as)").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
    };
    for method in ["ListActivatableNames", "ListNames"] {
        if let Ok(reply) = call(method).await {
            let names: Vec<String> = reply.child_value(0).get().unwrap_or_default();
            if names.iter().any(|n| n == app) {
                return true;
            }
        }
    }
    false
}
