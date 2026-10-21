pub mod audio;
pub mod bluetooth;
pub mod display;
pub mod input;
pub mod power;
pub mod usb;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

/// Runs `rescan` when `widget` is shown and every 2 seconds while it stays
/// shown. sysfs has no change signal Openhub can watch from the sandbox,
/// and a hidden page costs nothing this way.
pub fn rescan_while_mapped(widget: &impl IsA<gtk::Widget>, rescan: impl Fn() + 'static) {
    let rescan = Rc::new(rescan);
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    widget.connect_map(glib::clone!(
        #[strong]
        rescan,
        #[strong]
        timer,
        move |_| {
            rescan();
            let rescan = rescan.clone();
            let source = glib::timeout_add_seconds_local(2, move || {
                rescan();
                glib::ControlFlow::Continue
            });
            timer.replace(Some(source));
        }
    ));
    widget.connect_unmap(move |_| {
        if let Some(source) = timer.take() {
            source.remove();
        }
    });
}
