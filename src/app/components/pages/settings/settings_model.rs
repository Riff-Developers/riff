use crate::app::state::Action;
use crate::app::state::PlaybackAction;
use crate::app::{AppAction, AppModel, Dispatcher};
use crate::settings::RiffSettings;
use gettextrs::gettext;
use std::cell::RefCell;
use std::rc::Rc;

pub struct SettingsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    applied: RefCell<RiffSettings>,
}

impl SettingsModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
            applied: RefCell::new(RiffSettings::new_from_gsettings().unwrap_or_default()),
        }
    }

    /// Reloads the player if a setting changed that it can't pick up live.
    /// EQ, mono, pan and pitch are pushed live by the GSettings watcher in
    /// PlayerNotifier and never need a reload.
    pub fn apply_settings(&self) {
        let new_settings = RiffSettings::new_from_gsettings().unwrap_or_default();
        let applied = self.applied.replace(new_settings.clone());
        if applied
            .player_settings
            .requires_reload(&new_settings.player_settings)
        {
            self.dispatcher.dispatch_many(vec![
                PlaybackAction::Stop.into(),
                AppAction::Core(Action::ReloadPlayerSettings),
            ]);
        }
    }

    pub fn show_restart_required(&self) {
        self.dispatcher.dispatch(AppAction::ShowRestartRequired);
    }

    /// Clear the on-disk and in-memory data caches, plus librespot's
    /// downloaded audio cache, then notify the user.
    pub fn clear_cache(&self) {
        let api = self.app_model.api();
        let dispatcher = self.dispatcher.clone();
        tokio::spawn(async move {
            api.clear_user_cache().await;
            riff_playback::player::clear_audio_cache();
            // Translators: Toast shown after the user clears the cache in settings.
            dispatcher.dispatch(AppAction::ShowNotification(gettext("Cache cleared")));
        });
    }

    /// The settings the player on this computer supports.
    pub fn settings_support(&self) -> riff_config::SettingsSupport {
        riff_playback::player::Engine::chosen().settings_support()
    }

    /// Whether the logged-in Spotify account has locked its explicit content
    /// filter (e.g. via a family plan parental control). When locked, the user
    /// cannot disable explicit-track skipping in Riff.
    pub fn explicit_filter_locked(&self) -> bool {
        self.app_model.get_state().playback.explicit_filter_locked()
    }
}
