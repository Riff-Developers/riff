mod key;
mod support;
pub use key::{Setting, SettingSet, SettingsChanged};
pub use support::{Choice, SettingsSupport, Unavailable, Values};

#[cfg(feature = "gio")]
mod store;
#[cfg(feature = "gio")]
pub use store::{
    megabytes, CloseWindowBehavior, PinnedKind, PinnedObject, SetResult, SettingsStore,
    WindowGeometry,
};
