use gettextrs::gettext;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use crate::app::components::EventListener;
use crate::app::{AppEvent, AppModel};

use riff_config::{Setting, SettingsStore};

/// Pure decision helper: whether a suspend inhibitor should be held.
///
/// An inhibitor is held only while audio is actively playing and the user has
/// enabled the "keep device awake" preference.
fn should_inhibit(enabled: bool, playing: bool) -> bool {
    enabled && playing
}

pub struct SuspendInhibitor {
    window: libadwaita::ApplicationWindow,
    app_model: Rc<AppModel>,
    settings: SettingsStore,
    cookie: Cell<Option<u32>>,
}

impl SuspendInhibitor {
    pub fn new(window: libadwaita::ApplicationWindow, app_model: Rc<AppModel>) -> Self {
        let settings = SettingsStore::new();

        let inhibitor = Self {
            window,
            app_model,
            settings,
            cookie: Cell::new(None),
        };

        inhibitor.refresh();
        inhibitor
    }

    /// Applies the inhibit decision for the current preference and playback.
    fn refresh(&self) {
        let enabled = self.settings.bool(Setting::InhibitSuspend);
        let playing = self.app_model.get_state().playback.is_playing();
        self.apply(should_inhibit(enabled, playing));
    }

    /// Acquire or release the inhibitor to match `desired`, guarding against
    /// double-acquire and double-release.
    fn apply(&self, desired: bool) {
        let (window, cookie) = (&self.window, &self.cookie);
        let held = cookie.get().is_some();
        if desired == held {
            return;
        }

        let Some(app) = window.application() else {
            return;
        };

        if desired {
            let new_cookie = app.inhibit(
                Some(window),
                gtk::ApplicationInhibitFlags::SUSPEND,
                // Translators: Reason shown to the session manager for keeping
                // the device awake while music plays.
                Some(&gettext("Playing music")),
            );
            if new_cookie != 0 {
                cookie.set(Some(new_cookie));
            } else {
                log::warn!("Failed to acquire suspend inhibitor");
            }
        } else if let Some(old_cookie) = cookie.take() {
            app.uninhibit(old_cookie);
        }
    }
}

impl EventListener for SuspendInhibitor {
    fn on_event(&mut self, event: &AppEvent) {
        let suspend_changed = event
            .settings()
            .is_some_and(|changed| changed.contains(Setting::InhibitSuspend));
        if event.playback().is_some() || suspend_changed {
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_inhibit;

    #[test]
    fn inhibits_only_when_enabled_and_playing() {
        assert!(should_inhibit(true, true));
    }

    #[test]
    fn does_not_inhibit_when_paused_or_stopped() {
        assert!(!should_inhibit(true, false));
    }

    #[test]
    fn does_not_inhibit_when_disabled_while_playing() {
        assert!(!should_inhibit(false, true));
    }

    #[test]
    fn does_not_inhibit_when_disabled_and_not_playing() {
        assert!(!should_inhibit(false, false));
    }
}
