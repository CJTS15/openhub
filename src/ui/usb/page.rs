use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::UsbDevice;
use crate::usb::UsbManager;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/usb_page.ui")]
    pub struct UsbPage {
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
            let model = gtk::FilterListModel::new(Some(self.manager.devices()), Some(filter));
            self.device_list.bind_model(
                Some(&model),
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_panic]
                    move |item| obj.row(item.downcast_ref().unwrap())
                ),
            );

            // A rescan rebuilds every row, hub children included, with the new setting.
            self.show_internal.connect_active_notify(glib::clone!(
                #[weak]
                obj,
                move |_| obj.imp().manager.refresh()
            ));
            // ponytail: scans when shown; M6 adds the 2-second rescan while visible.
            obj.connect_map(|page| page.imp().manager.refresh());
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

    /// A plain row, or a collapsed expander for a hub with visible children.
    fn row(&self, device: &UsbDevice) -> gtk::Widget {
        let children: Vec<UsbDevice> = device
            .children()
            .iter::<UsbDevice>()
            .flatten()
            .filter(|child| self.shows(child))
            .collect();
        let icon = gtk::Image::from_icon_name(&device.icon_name());
        if children.is_empty() {
            let row = adw::ActionRow::builder()
                .title(device.title())
                .subtitle(device.subtitle())
                .use_markup(false)
                .build();
            row.add_prefix(&icon);
            return row.upcast();
        }
        let row = adw::ExpanderRow::builder()
            .title(device.title())
            .subtitle(device.subtitle())
            .use_markup(false)
            .build();
        row.add_prefix(&icon);
        for child in &children {
            row.add_row(&self.row(child));
        }
        row.upcast()
    }
}
