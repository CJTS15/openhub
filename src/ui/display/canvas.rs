use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};

/// One monitor on the canvas: where it sits on the desktop, in layout pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub connector: String,
    pub number: usize,
    pub name: String,
    pub rect: (i32, i32, i32, i32),
    pub primary: bool,
}

type Dropped = Box<dyn Fn(&str, i32, i32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Canvas {
        pub items: RefCell<Vec<Item>>,
        /// The monitor being dragged, and how far, in widget pixels.
        pub dragging: Cell<Option<usize>>,
        pub offset: Cell<(f64, f64)>,
        pub dropped: RefCell<Option<Dropped>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "OpenhubDisplayCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Canvas {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_cursor_from_name(Some("grab"));
            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin(glib::clone!(
                #[weak]
                obj,
                move |_, x, y| {
                    let imp = obj.imp();
                    imp.dragging.set(obj.item_at(x, y));
                    imp.offset.set((0.0, 0.0));
                }
            ));
            drag.connect_drag_update(glib::clone!(
                #[weak]
                obj,
                move |_, dx, dy| {
                    if obj.imp().dragging.get().is_some() {
                        obj.imp().offset.set((dx, dy));
                        obj.queue_draw();
                    }
                }
            ));
            drag.connect_drag_end(glib::clone!(
                #[weak]
                obj,
                move |_, dx, dy| obj.drop_at(dx, dy)
            ));
            obj.add_controller(drag);
        }
    }

    impl WidgetImpl for Canvas {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (200, 480, -1, -1),
                _ => (200, 220, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    /// The arrangement: monitors drawn to scale; drag one and drop it
    /// beside another.
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Canvas {
    fn default() -> Self {
        glib::Object::new()
    }
}

const PADDING: f64 = 24.0;
const GAP: f32 = 4.0;

impl Canvas {
    pub fn set_items(&self, items: Vec<Item>) {
        let description: Vec<String> = items
            .iter()
            .map(|item| format!("{} {}", item.number, item.name))
            .collect();
        self.update_property(&[gtk::accessible::Property::Label(&format!(
            "Arrangement: {}",
            description.join(", ")
        ))]);
        self.imp().items.replace(items);
        self.imp().dragging.set(None);
        self.queue_draw();
    }

    /// Called with a monitor's connector and where it was dropped, in
    /// layout pixels.
    pub fn connect_dropped(&self, dropped: impl Fn(&str, i32, i32) + 'static) {
        self.imp().dropped.replace(Some(Box::new(dropped)));
    }

    /// Layout pixels per widget pixel, and where the layout's 0,0 is drawn.
    fn transform(&self) -> (f64, f64, f64) {
        let items = self.imp().items.borrow();
        let right = items
            .iter()
            .map(|i| i.rect.0 + i.rect.2)
            .max()
            .unwrap_or(1)
            .max(1);
        let bottom = items
            .iter()
            .map(|i| i.rect.1 + i.rect.3)
            .max()
            .unwrap_or(1)
            .max(1);
        let (width, height) = (f64::from(self.width()), f64::from(self.height()));
        let scale = ((width - 2.0 * PADDING) / f64::from(right))
            .min((height - 2.0 * PADDING) / f64::from(bottom))
            .max(1e-3);
        // Centred.
        let x = (width - f64::from(right) * scale) / 2.0;
        let y = (height - f64::from(bottom) * scale) / 2.0;
        (scale, x, y)
    }

    fn item_at(&self, x: f64, y: f64) -> Option<usize> {
        let (scale, ox, oy) = self.transform();
        self.imp().items.borrow().iter().position(|item| {
            let (rx, ry, rw, rh) = item.rect;
            let left = ox + f64::from(rx) * scale;
            let top = oy + f64::from(ry) * scale;
            (left..left + f64::from(rw) * scale).contains(&x)
                && (top..top + f64::from(rh) * scale).contains(&y)
        })
    }

    fn drop_at(&self, dx: f64, dy: f64) {
        let imp = self.imp();
        let Some(index) = imp.dragging.take() else {
            return;
        };
        let (scale, _, _) = self.transform();
        let item = imp.items.borrow()[index].clone();
        let x = item.rect.0 + (dx / scale).round() as i32;
        let y = item.rect.1 + (dy / scale).round() as i32;
        self.queue_draw();
        if let Some(dropped) = imp.dropped.borrow().as_ref() {
            dropped(&item.connector, x, y);
        }
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let (scale, ox, oy) = self.transform();
        let fg = self.color();
        let shade = |alpha: f32| gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), alpha);
        let (dx, dy) = imp.offset.get();
        for (index, item) in imp.items.borrow().iter().enumerate() {
            let dragged = imp.dragging.get() == Some(index);
            let (rx, ry, rw, rh) = item.rect;
            let (mut left, mut top) = (ox + f64::from(rx) * scale, oy + f64::from(ry) * scale);
            if dragged {
                left += dx;
                top += dy;
            }
            let bounds = graphene::Rect::new(
                left as f32 + GAP / 2.0,
                top as f32 + GAP / 2.0,
                (f64::from(rw) * scale) as f32 - GAP,
                (f64::from(rh) * scale) as f32 - GAP,
            );
            let rounded = gsk::RoundedRect::from_rect(bounds, 8.0);
            let fill = match (dragged, item.primary) {
                (true, _) => 0.30,
                (false, true) => 0.20,
                (false, false) => 0.10,
            };
            snapshot.push_rounded_clip(&rounded);
            snapshot.append_color(&shade(fill), &bounds);
            snapshot.pop();
            snapshot.append_border(&rounded, &[1.0; 4], &[shade(0.45); 4]);

            // Number large, name below, centred.
            let label = format!(
                "<span size='x-large' weight='bold'>{}</span>\n{}",
                item.number,
                glib::markup_escape_text(&item.name)
            );
            let layout = self.create_pango_layout(None);
            layout.set_markup(&label);
            layout.set_alignment(pango::Alignment::Center);
            layout.set_width((bounds.width() * pango::SCALE as f32) as i32);
            layout.set_ellipsize(pango::EllipsizeMode::End);
            let (_, text) = layout.pixel_extents();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                bounds.x(),
                bounds.y() + (bounds.height() - text.height() as f32) / 2.0,
            ));
            snapshot.append_layout(&layout, &fg);
            snapshot.restore();
        }
    }
}
