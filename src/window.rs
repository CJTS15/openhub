use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::ui::audio::AudioPage;
use crate::ui::bluetooth::BluetoothPage;
use crate::ui::display::DisplayPage;
use crate::ui::input::InputPage;
use crate::ui::power::PowerPage;
use crate::ui::system::SystemPage;
use crate::ui::usb::UsbPage;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/window.ui")]
    pub struct OpenhubWindow {
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub categories: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub content_page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub category_stack: TemplateChild<gtk::Stack>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpenhubWindow {
        const NAME: &'static str = "OpenhubWindow";
        type Type = super::OpenhubWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            SystemPage::ensure_type();
            BluetoothPage::ensure_type();
            UsbPage::ensure_type();
            PowerPage::ensure_type();
            InputPage::ensure_type();
            AudioPage::ensure_type();
            DisplayPage::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl OpenhubWindow {
        #[template_callback]
        fn on_category_activated(&self, row: &gtk::ListBoxRow) {
            // Each sidebar row's widget name is its stack page's name.
            let stack = &self.category_stack;
            stack.set_visible_child_name(&row.widget_name());
            if let Some(child) = stack.visible_child() {
                let title = stack.page(&child).title().unwrap_or_default();
                self.content_page.set_title(&title);
            }
            // On narrow windows only one pane is visible; move to the content.
            self.split_view.set_show_content(true);
        }
    }

    impl ObjectImpl for OpenhubWindow {
        fn constructed(&self) {
            self.parent_constructed();
            if let Some(row) = self.categories.row_at_index(0) {
                self.categories.select_row(Some(&row));
            }
        }
    }
    impl WidgetImpl for OpenhubWindow {}
    impl WindowImpl for OpenhubWindow {}
    impl ApplicationWindowImpl for OpenhubWindow {}
    impl AdwApplicationWindowImpl for OpenhubWindow {}
}

glib::wrapper! {
    pub struct OpenhubWindow(ObjectSubclass<imp::OpenhubWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl OpenhubWindow {
    pub fn new<P: glib::object::IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }
}
