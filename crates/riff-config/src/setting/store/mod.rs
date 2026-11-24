use std::cell::Cell;
use std::rc::Rc;

use gio::glib;
use gio::prelude::*;

use super::{Setting, SettingSet, SettingsChanged};

mod account;
mod cache;
mod pins;
mod view;
mod window;

pub use cache::megabytes;
pub use pins::{PinnedKind, PinnedObject};
pub use window::{CloseWindowBehavior, WindowGeometry};

const SCHEMA_ID: &str = "dev.diegovsky.Riff";

pub type SetResult = Result<(), glib::BoolError>;

#[derive(Clone, Debug)]
pub struct SettingsStore(gio::Settings);

impl Default for SettingsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsStore {
    pub fn new() -> Self {
        Self(gio::Settings::new(SCHEMA_ID))
    }

    pub fn bool(&self, setting: Setting) -> bool {
        self.0.boolean(setting.key())
    }

    pub fn set_bool(&self, setting: Setting, value: bool) -> SetResult {
        self.0.set_boolean(setting.key(), value)
    }

    pub fn int(&self, setting: Setting) -> i32 {
        self.0.int(setting.key())
    }

    pub fn set_int(&self, setting: Setting, value: i32) -> SetResult {
        self.0.set_int(setting.key(), value)
    }

    pub fn uint(&self, setting: Setting) -> u32 {
        self.0.uint(setting.key())
    }

    pub fn set_uint(&self, setting: Setting, value: u32) -> SetResult {
        self.0.set_uint(setting.key(), value)
    }

    pub fn double(&self, setting: Setting) -> f64 {
        self.0.double(setting.key())
    }

    pub fn set_double(&self, setting: Setting, value: f64) -> SetResult {
        self.0.set_double(setting.key(), value)
    }

    pub fn string(&self, setting: Setting) -> String {
        self.0.string(setting.key()).into()
    }

    pub fn set_string(&self, setting: Setting, value: &str) -> SetResult {
        self.0.set_string(setting.key(), value)
    }

    pub fn enum_(&self, setting: Setting) -> i32 {
        self.0.enum_(setting.key())
    }

    pub fn set_enum(&self, setting: Setting, value: i32) -> SetResult {
        self.0.set_enum(setting.key(), value)
    }

    pub fn value(&self, setting: Setting) -> glib::Variant {
        self.0.value(setting.key())
    }

    pub fn bind<'a, O: IsA<glib::Object>>(
        &'a self,
        setting: Setting,
        object: &'a O,
        property: &'a str,
    ) -> gio::BindingBuilder<'a> {
        self.0.bind(setting.key(), object, property)
    }

    pub fn unbind<O: IsA<glib::Object>>(object: &O, property: &str) {
        gio::Settings::unbind(object, property);
    }

    pub fn batch(&self, changes: impl FnOnce(&Self) -> SetResult) -> SetResult {
        let delayed = Self::new();
        delayed.0.delay();
        let result = changes(&delayed);
        match result {
            Ok(()) => delayed.0.apply(),
            Err(_) => delayed.0.revert(),
        }
        result
    }

    pub fn watch(&self, on_change: impl Fn(SettingsChanged) + 'static) -> glib::SignalHandlerId {
        let pending = Rc::new(Cell::new(SettingSet::default()));
        let on_change = Rc::new(on_change);
        self.0.connect_changed(None, move |_, key| {
            let Some(setting) = Setting::from_key(key) else {
                warn!("A setting without a name changed: {key}");
                return;
            };
            let mut changed = pending.get();
            let first = changed.is_empty();
            changed.insert(setting);
            pending.set(changed);
            if first {
                let pending = Rc::clone(&pending);
                let on_change = Rc::clone(&on_change);
                glib::idle_add_local_once(move || {
                    on_change(SettingsChanged {
                        changed: pending.take(),
                    })
                });
            }
        })
    }
}
