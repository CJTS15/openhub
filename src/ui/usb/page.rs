use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::device_dialog;
use crate::model::UsbDevice;
use crate::usb::{self, UsbManager};

/// How often the page rescans sysfs while it's on screen.
const RESCAN_SECONDS: u32 = 2;

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
        pub rescan: RefCell<Option<glib::SourceId>>,
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
                    move |item| obj.row(item.downcast_ref().unwrap())
                ),
            );
            // Hub rows listen for this too and redo their own children.
            self.show_internal
                .connect_active_notify(move |_| filter.changed(gtk::FilterChange::Different));

            obj.connect_map(|page| {
                page.imp().manager.refresh();
                let rescan = glib::timeout_add_seconds_local(
                    RESCAN_SECONDS,
                    glib::clone!(
                        #[weak]
                        page,
                        #[upgrade_or]
                        glib::ControlFlow::Break,
                        move || {
                            page.imp().manager.refresh();
                            glib::ControlFlow::Continue
                        }
                    ),
                );
                page.imp().rescan.replace(Some(rescan));
            });
            obj.connect_unmap(|page| {
                if let Some(rescan) = page.imp().rescan.take() {
                    rescan.remove();
                }
            });
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

    /// A row that follows `device`: a plain row, or an expander for a hub.
    fn row(&self, device: &UsbDevice) -> gtk::Widget {
        let icon = gtk::Image::new();
        device
            .bind_property("icon-name", &icon, "icon-name")
            .sync_create()
            .build();
        if !device.hub() {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .activatable(true)
                .build();
            bind_text(device, &row);
            row.add_prefix(&icon);
            if !device.disks().is_empty() {
                row.add_suffix(&self.eject_button(device));
            }
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.connect_activated(glib::clone!(
                #[weak(rename_to = page)]
                self,
                #[weak]
                device,
                move |_| page.show_details(&device)
            ));
            return row.upcast();
        }

        let row = adw::ExpanderRow::builder().use_markup(false).build();
        bind_text(device, &row);
        row.add_prefix(&icon);
        let details = icon_button("go-next-symbolic", "Details");
        details.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |_| page.show_details(&device)
        ));
        row.add_suffix(&details);

        // Rows for what's plugged in, redone when that changes or the
        // internal-devices switch flips. Rebuilding resets nested hubs'
        // expansion, which only happens when something under this hub changes.
        let shown: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
        let rebuild: Rc<dyn Fn()> = Rc::new(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            row,
            #[weak]
            device,
            #[strong]
            shown,
            move || {
                for child in shown.take() {
                    row.remove(&child);
                }
                for child in device.children().iter::<UsbDevice>().flatten() {
                    if page.shows(&child) {
                        let child_row = page.row(&child);
                        row.add_row(&child_row);
                        shown.borrow_mut().push(child_row);
                    }
                }
                row.set_enable_expansion(!shown.borrow().is_empty());
            }
        ));
        rebuild();
        let on_items = device.children().connect_items_changed(glib::clone!(
            #[strong]
            rebuild,
            move |_, _, _, _| rebuild()
        ));
        let on_switch = self
            .imp()
            .show_internal
            .connect_active_notify(move |_| rebuild());
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
        row.upcast()
    }
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

fn bind_text(device: &UsbDevice, row: &impl IsA<adw::PreferencesRow>) {
    device
        .bind_property("title", row.as_ref(), "title")
        .sync_create()
        .build();
    device
        .bind_property("subtitle", row.as_ref(), "subtitle")
        .sync_create()
        .build();
}
