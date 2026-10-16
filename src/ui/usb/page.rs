use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::device_dialog;
use crate::model::{PowerDevice, UsbDevice};
use crate::upower::{PowerManager, convert};
use crate::usb::{self, UsbManager};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/usb_page.ui")]
    pub struct UsbPage {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub show_internal: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub device_list: TemplateChild<gtk::ListBox>,
        pub manager: UsbManager,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UsbPage {
        const NAME: &'static str = "OpenhubUsbPage";
        type Type = super::UsbPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for UsbPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let filter = gtk::CustomFilter::new(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                true,
                move |item| obj.shows(item.downcast_ref().unwrap())
            ));
            let model =
                gtk::FilterListModel::new(Some(self.manager.devices()), Some(filter.clone()));
            self.device_list.bind_model(
                Some(&model),
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_panic]
                    move |item| obj.top_level_row(item.downcast_ref().unwrap())
                ),
            );
            // Hub rows listen for this too and redo their own children.
            self.show_internal
                .connect_active_notify(move |_| filter.changed(gtk::FilterChange::Different));

            // Batteries come from UPower; they're matched to rows on each rescan
            // and whenever UPower's device list changes.
            PowerManager::shared()
                .peripherals()
                .connect_items_changed(glib::clone!(
                    #[weak]
                    obj,
                    move |_, _, _, _| obj.update_batteries()
                ));

            crate::ui::rescan_while_mapped(
                &*obj,
                glib::clone!(
                    #[weak]
                    obj,
                    move || obj.rescan()
                ),
            );
        }
    }

    impl WidgetImpl for UsbPage {}
    impl BinImpl for UsbPage {}
}

glib::wrapper! {
    pub struct UsbPage(ObjectSubclass<imp::UsbPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl UsbPage {
    fn rescan(&self) {
        self.imp().manager.refresh();
        self.update_batteries();
    }

    /// Puts the batteries UPower reports through USB receivers on the rows
    /// of those receivers. UsbManager doesn't know about UPower; the two
    /// meet only here, by the USB device's sysfs name.
    fn update_batteries(&self) {
        let mut by_usb: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();
        for battery in PowerManager::shared()
            .peripherals()
            .iter::<PowerDevice>()
            .flatten()
        {
            let usb_device = battery.usb_device();
            if !usb_device.is_empty() {
                by_usb.entry(usb_device).or_default().push((
                    battery.title(),
                    battery.level_text(),
                    battery.fraction(),
                ));
            }
        }
        let mut pending: Vec<UsbDevice> = self.imp().manager.devices().iter().flatten().collect();
        while let Some(device) = pending.pop() {
            pending.extend(device.children().iter::<UsbDevice>().flatten());
            let batteries = by_usb.remove(&device.name()).unwrap_or_default();
            let (row, details) = convert::usb_battery_summary(&batteries);
            if device.battery() != row {
                device.set_battery(row);
            }
            if device.batteries() != details {
                device.set_batteries(details);
            }
        }
    }

    fn shows(&self, device: &UsbDevice) -> bool {
        self.imp().show_internal.is_active() || !device.internal()
    }

    fn show_details(&self, device: &UsbDevice) {
        device_dialog::new(device).present(Some(self));
    }

    fn toast(&self, title: &str) {
        let toast = adw::Toast::builder().title(title).use_markup(false).build();
        self.imp().toast_overlay.add_toast(toast);
    }

    /// Unmounts and powers off a storage device, then says whether it's safe
    /// to unplug. The row goes away on the next rescan once it powers off.
    fn eject_button(&self, device: &UsbDevice) -> gtk::Button {
        let button = icon_button("media-eject-symbolic", "Eject");
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |button| {
                // Flushing and spinning down a hard drive takes seconds; show it's working.
                button.set_sensitive(false);
                button.set_child(Some(&gtk::Spinner::builder().spinning(true).build()));
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    page,
                    #[weak]
                    device,
                    #[weak]
                    button,
                    async move {
                        let name = device.title();
                        let manager = &page.imp().manager;
                        match manager.eject(&device).await {
                            Ok(()) => {
                                // Powering off removes it from sysfs; drop the row now.
                                manager.refresh();
                                page.toast(&format!("{name} can be removed"));
                            }
                            Err(err) if usb::is_busy(&err) => page.toast(&format!(
                                "{name} is in use. Close any open files on it and try again."
                            )),
                            Err(err) => {
                                glib::g_warning!("openhub", "Can't eject {}: {err}", device.name());
                                page.toast(&format!("Couldn't eject {name}"));
                            }
                        }
                        button.set_icon_name("media-eject-symbolic");
                        button.set_sensitive(true);
                    }
                ));
            }
        ));
        button
    }

    fn visible_children(&self, device: &UsbDevice) -> Vec<UsbDevice> {
        device
            .children()
            .iter::<UsbDevice>()
            .flatten()
            .filter(|child| self.shows(child))
            .collect()
    }

    /// A row for a device on a root port; replacing its store item rebuilds it.
    fn top_level_row(&self, device: &UsbDevice) -> gtk::Widget {
        let reshape: Rc<dyn Fn()> = Rc::new(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move || {
                let devices = page.imp().manager.devices();
                if let Some(position) = devices.find(&device) {
                    devices.splice(position, 1, std::slice::from_ref(&device));
                }
            }
        ));
        self.row(device, None, reshape)
    }

    /// A row that follows `device`. A hub with something visible plugged in
    /// is an expander; anything else, an empty hub included, is a plain row.
    /// `through` is the hub it's plugged into, shown in the subtitle.
    /// `reshape` rebuilds this row in its list, for when a hub gains its
    /// first visible device or loses its last.
    fn row(
        &self,
        device: &UsbDevice,
        through: Option<&UsbDevice>,
        reshape: Rc<dyn Fn()>,
    ) -> gtk::Widget {
        let expanded_shape = !self.visible_children(device).is_empty();
        let (row, rebuild_children): (gtk::Widget, Option<Rc<dyn Fn()>>) = if expanded_shape {
            let (row, rebuild) = self.hub_row(device);
            (row.upcast(), Some(rebuild))
        } else {
            (self.device_row(device).upcast(), None)
        };
        bind_text(device, row.downcast_ref().unwrap(), through);
        if !device.hub() {
            return row;
        }

        // Follow what's plugged in and the internal-devices switch.
        let update: Rc<dyn Fn()> = Rc::new(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move || {
                if page.visible_children(&device).is_empty() == expanded_shape {
                    reshape();
                } else if let Some(rebuild) = &rebuild_children {
                    rebuild();
                }
            }
        ));
        let on_items = device.children().connect_items_changed(glib::clone!(
            #[strong]
            update,
            move |_, _, _, _| update()
        ));
        let on_switch = self
            .imp()
            .show_internal
            .connect_active_notify(move |_| update());
        // Disconnect both when the row goes away, or they'd outlive it.
        let handlers = RefCell::new(Some((on_items, on_switch)));
        row.connect_destroy(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |_| {
                if let Some((on_items, on_switch)) = handlers.take() {
                    device.children().disconnect(on_items);
                    page.imp().show_internal.disconnect(on_switch);
                }
            }
        ));
        row
    }

    /// A plain row: opens details, with Eject while the device has disks.
    fn device_row(&self, device: &UsbDevice) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .use_markup(false)
            .activatable(true)
            .build();
        row.add_prefix(&device_icon(device));
        // The disk shows up a second or two after the USB device, so a
        // row can exist before it does; follow the property, not its first value.
        let eject = self.eject_button(device);
        device
            .bind_property("disks", &eject, "visible")
            .transform_to(|_, disks: Vec<String>| Some(!disks.is_empty()))
            .sync_create()
            .build();
        row.add_suffix(&eject);
        row.connect_activated(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |_| page.show_details(&device)
        ));
        row
    }

    /// An expander listing what's plugged into the hub, and the function that
    /// redoes that list. Redoing it resets nested hubs' expansion, which only
    /// happens when something under this hub changes.
    fn hub_row(&self, device: &UsbDevice) -> (adw::ExpanderRow, Rc<dyn Fn()>) {
        let row = adw::ExpanderRow::builder().use_markup(false).build();
        row.add_prefix(&device_icon(device));
        let details = icon_button("help-about-symbolic", "Details");
        details.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |_| page.show_details(&device)
        ));
        row.add_suffix(&details);

        let shown: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
        self.fill_hub(&row, device, &shown);
        let rebuild: Rc<dyn Fn()> = Rc::new(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            row,
            #[weak]
            device,
            #[strong]
            shown,
            move || page.fill_hub(&row, &device, &shown)
        ));
        (row, rebuild)
    }

    /// Redoes the rows for what's plugged into the hub. Their `reshape`
    /// redoes this list again; it holds only weak references, so a child
    /// can't keep a removed hub row alive.
    fn fill_hub(
        &self,
        row: &adw::ExpanderRow,
        device: &UsbDevice,
        shown: &Rc<RefCell<Vec<gtk::Widget>>>,
    ) {
        for child in shown.take() {
            row.remove(&child);
        }
        let reshape: Rc<dyn Fn()> = Rc::new(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            row,
            #[weak]
            device,
            #[weak]
            shown,
            move || page.fill_hub(&row, &device, &shown)
        ));
        for child in self.visible_children(device) {
            let child_row = self.row(&child, Some(device), reshape.clone());
            row.add_row(&child_row);
            shown.borrow_mut().push(child_row);
        }
    }
}

fn device_icon(device: &UsbDevice) -> gtk::Image {
    let icon = gtk::Image::new();
    device
        .bind_property("icon-name", &icon, "icon-name")
        .sync_create()
        .build();
    icon
}

/// A flat icon button; `label` is its tooltip and screen-reader name.
fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(label)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Title, and the vendor as subtitle; a device behind a hub also says
/// which hub, so it doesn't read as if it were on its own port.
fn bind_text(device: &UsbDevice, row: &adw::PreferencesRow, through: Option<&UsbDevice>) {
    device
        .bind_property("title", row, "title")
        .sync_create()
        .build();
    let hub = through.map(UsbDevice::title);
    let property =
        |name| gtk::PropertyExpression::new(UsbDevice::static_type(), gtk::Expression::NONE, name);
    gtk::ClosureExpression::with_callback(
        [property("subtitle"), property("battery")],
        move |values| {
            let vendor: String = values[1].get().unwrap_or_default();
            let battery: String = values[2].get().unwrap_or_default();
            subtitle(&vendor, &battery, hub.as_deref())
        },
    )
    .bind(row, "subtitle", Some(device));
}

/// "Vendor · Connected through USB2.1 Hub · 80% Battery", without the
/// parts a device doesn't have.
fn subtitle(vendor: &str, battery: &str, hub: Option<&str>) -> String {
    let through = hub.map(|hub| format!("Connected through {hub}"));
    [vendor, through.as_deref().unwrap_or_default(), battery]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}
