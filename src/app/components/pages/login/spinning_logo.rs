use std::cell::{Cell, OnceCell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{graphene, gsk};
use libadwaita::prelude::AnimationExt;

const PULSE_SECONDS: f64 = 3.0;
const PULSE_DEGREES: f64 = 720.0;
const PULSE_EASING: (f64, f64, f64, f64) = (0.75, 0.03, 0.25, 0.97);
const GLIDE_MS: u32 = 800;
const SETTLE_SECONDS: f64 = 0.8;
const SETTLE_MAX_SECONDS: f64 = 1.2;
const REST_STEP: f64 = 45.0;

/// Where the logo's pictures are; each is drawn 128 pixels square.
const LOGO_RESOURCES: &str = "/dev/diegovsky/Riff/logo";

struct Layer {
    name: &'static str,
    pivot: (f32, f32),
}

/// Bottom to top.
const LAYERS: [Layer; 2] = [
    Layer {
        name: "ring-shadow",
        pivot: (64.0, 66.604),
    },
    Layer {
        name: "ring",
        pivot: (64.0, 62.011),
    },
];

/// A layer's outline and its light.
#[derive(Debug)]
struct LayerArt {
    pivot: (f32, f32),
    outline: gtk::Svg,
    light: gtk::Svg,
}

#[derive(Debug)]
struct Art {
    layers: Vec<LayerArt>,
    middle: gtk::Svg,
}

impl Art {
    fn load() -> Self {
        let svg = |name: &str| gtk::Svg::from_resource(&format!("{LOGO_RESOURCES}/{name}.svg"));
        Self {
            layers: LAYERS
                .iter()
                .map(|layer| LayerArt {
                    pivot: layer.pivot,
                    outline: svg(layer.name),
                    light: svg(&format!("{}-light", layer.name)),
                })
                .collect(),
            middle: svg("middle"),
        }
    }
}

fn pulse_ease(progress: f64) -> f64 {
    let (x1, y1, x2, y2) = PULSE_EASING;
    let bezier = |t: f64, p1: f64, p2: f64| {
        3.0 * (1.0 - t).powi(2) * t * p1 + 3.0 * (1.0 - t) * t * t * p2 + t.powi(3)
    };
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (low + high) / 2.0;
        if bezier(mid, x1, x2) < progress {
            low = mid;
        } else {
            high = mid;
        }
    }
    bezier((low + high) / 2.0, y1, y2)
}

fn pulse_angle(seconds: f64) -> f64 {
    let pulses = seconds / PULSE_SECONDS;
    (pulses.floor() + pulse_ease(pulses.fract())) * PULSE_DEGREES
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Still,
    Spinning {
        since: i64,
        from: f64,
    },
    Settling {
        since: i64,
        from: f64,
        speed: f64,
        to: f64,
        seconds: f64,
    },
}

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::SpinningLogo)]
    pub struct SpinningLogo {
        #[property(get, set = Self::set_pixel_size)]
        pub pixel_size: Cell<i32>,
        pub angle: Cell<f64>,
        pub angle_time: Cell<i64>,
        pub motion: Cell<Motion>,
        pub tick: RefCell<Option<gtk::TickCallbackId>>,
        pub offset: Cell<f64>,
        pub glide_from: Cell<Option<f64>>,
        pub glide: RefCell<Option<libadwaita::TimedAnimation>>,
        pub(super) art: OnceCell<Art>,
    }

    impl Default for SpinningLogo {
        fn default() -> Self {
            Self {
                pixel_size: Cell::new(128),
                angle: Cell::default(),
                angle_time: Cell::default(),
                motion: Cell::new(Motion::Still),
                tick: RefCell::default(),
                offset: Cell::default(),
                glide_from: Cell::default(),
                glide: RefCell::default(),
                art: OnceCell::new(),
            }
        }
    }

    impl SpinningLogo {
        fn set_pixel_size(&self, size: i32) {
            self.pixel_size.set(size);
            self.obj().queue_resize();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SpinningLogo {
        const NAME: &'static str = "RiffSpinningLogo";
        type Type = super::SpinningLogo;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SpinningLogo {
        fn dispose(&self) {
            if let Some(glide) = self.glide.take() {
                glide.skip();
            }
        }
    }

    impl WidgetImpl for SpinningLogo {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let size = self.pixel_size.get();
            (size, size, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(from) = self.glide_from.take() {
                self.obj().glide(from);
            }
        }

        fn map(&self) {
            self.parent_map();
            self.obj().update_tick();
        }

        fn unmap(&self) {
            if let Some(tick) = self.tick.take() {
                tick.remove();
            }
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct SpinningLogo(ObjectSubclass<imp::SpinningLogo>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SpinningLogo {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SpinningLogo {
    pub fn new(pixel_size: i32) -> Self {
        glib::Object::builder()
            .property("pixel-size", pixel_size)
            .build()
    }

    pub fn spin(&self) {
        if matches!(self.imp().motion.get(), Motion::Spinning { .. }) {
            return;
        }
        self.imp().motion.set(Motion::Spinning {
            since: self.now(),
            from: self.imp().angle.get(),
        });
        self.update_tick();
    }

    pub fn settle(&self) {
        let imp = self.imp();
        let Motion::Spinning { since, from } = imp.motion.get() else {
            return;
        };
        if !self.animates() {
            return self.rest();
        }
        let now = match imp.angle_time.get() {
            0 => self.now(),
            drawn => drawn,
        };
        let seconds = (now - since) as f64 / 1e6;
        let speed = (pulse_angle(seconds + 0.005) - pulse_angle(seconds - 0.005)) / 0.01;
        let angle = from + pulse_angle(seconds);
        let to = ((angle + speed * SETTLE_SECONDS / 2.0) / REST_STEP).ceil() * REST_STEP;
        let seconds = (2.0 * (to - angle) / speed.max(1.0)).min(SETTLE_MAX_SECONDS);
        imp.angle.set(angle);
        imp.motion.set(Motion::Settling {
            since: now,
            from: angle,
            speed,
            to,
            seconds,
        });
        self.update_tick();
    }

    pub fn rest(&self) {
        let imp = self.imp();
        imp.motion.set(Motion::Still);
        imp.angle.set(0.0);
        if let Some(tick) = imp.tick.take() {
            tick.remove();
        }
        self.queue_draw();
    }

    pub fn glide_from(&self, window_y: f64) {
        self.imp().glide_from.set(Some(window_y));
        self.queue_allocate();
    }

    pub fn window_y(&self) -> Option<f64> {
        let root = self.root()?;
        self.compute_point(&root, &graphene::Point::new(0.0, 0.0))
            .map(|point| f64::from(point.y()))
    }

    fn glide(&self, from: f64) {
        let Some(to) = self.window_y() else {
            return;
        };
        let target = libadwaita::CallbackAnimationTarget::new(glib::clone!(
            #[weak(rename_to = logo)]
            self,
            move |offset| {
                logo.imp().offset.set(offset);
                logo.queue_draw();
            }
        ));
        let glide = libadwaita::TimedAnimation::new(self, from - to, 0.0, GLIDE_MS, target);
        glide.set_easing(libadwaita::Easing::EaseInOutSine);
        self.imp().offset.set(from - to);
        self.queue_draw();
        glide.play();
        if let Some(old) = self.imp().glide.replace(Some(glide)) {
            old.pause();
        }
    }

    fn animates(&self) -> bool {
        self.settings().is_gtk_enable_animations()
    }

    fn now(&self) -> i64 {
        self.frame_clock()
            .map_or_else(glib::monotonic_time, |clock| clock.frame_time())
    }

    fn update_tick(&self) {
        let imp = self.imp();
        let moving = imp.motion.get() != Motion::Still;
        if !moving || !self.is_mapped() || imp.tick.borrow().is_some() {
            return;
        }
        let tick = self.add_tick_callback(|logo, clock| {
            logo.advance(clock.frame_time());
            if logo.imp().motion.get() == Motion::Still {
                logo.imp().tick.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        imp.tick.replace(Some(tick));
    }

    fn advance(&self, now: i64) {
        let imp = self.imp();
        imp.angle_time.set(now);
        match imp.motion.get() {
            Motion::Still => {}
            Motion::Spinning { since, from } => {
                imp.angle
                    .set(from + pulse_angle((now - since) as f64 / 1e6));
            }
            Motion::Settling {
                since,
                from,
                speed,
                to,
                seconds,
            } => {
                let s = ((now - since) as f64 / 1e6 / seconds).min(1.0);
                let angle = (2.0 * s.powi(3) - 3.0 * s * s + 1.0) * from
                    + (s.powi(3) - 2.0 * s * s + s) * speed * seconds
                    + (-2.0 * s.powi(3) + 3.0 * s * s) * to;
                imp.angle.set(angle);
                if s >= 1.0 {
                    imp.motion.set(Motion::Still);
                    imp.angle.set(to % 360.0);
                }
            }
        }
        self.queue_draw();
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let art = imp.art.get_or_init(Art::load);
        let size = imp.pixel_size.get() as f32;
        let angle = (imp.angle.get() % 360.0) as f32;

        snapshot.save();
        snapshot.translate(&graphene::Point::new(0.0, imp.offset.get() as f32));
        snapshot.scale(size / 128.0, size / 128.0);
        for layer in &art.layers {
            let (x, y) = layer.pivot;
            snapshot.push_mask(gsk::MaskMode::Alpha);
            snapshot.save();

            snapshot.translate(&graphene::Point::new(x, y));
            snapshot.rotate(angle);
            snapshot.translate(&graphene::Point::new(-x, -y));
            layer.outline.snapshot(snapshot, 128.0, 128.0);
            snapshot.restore();
            snapshot.pop();

            layer.light.snapshot(snapshot, 128.0, 128.0);
            snapshot.pop();
        }
        art.middle.snapshot(snapshot, 128.0, 128.0);
        snapshot.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pulse_turns_twice_and_starts_and_ends_slowly() {
        assert!(pulse_angle(0.0).abs() < 1e-6);
        assert!((pulse_angle(PULSE_SECONDS) - PULSE_DEGREES).abs() < 1e-6);
        assert!((pulse_angle(PULSE_SECONDS / 2.0) - PULSE_DEGREES / 2.0).abs() < 1.0);
        let speed = |t: f64| (pulse_angle(t + 0.001) - pulse_angle(t - 0.001)) / 0.002;
        assert!(speed(0.01) < 20.0, "{}", speed(0.01));
        let peak = speed(PULSE_SECONDS / 2.0);
        assert!((850.0..1000.0).contains(&peak), "{peak}");
    }
}
