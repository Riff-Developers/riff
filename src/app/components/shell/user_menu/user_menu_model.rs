use crate::app::components::dispatch_api_read;
use crate::app::state::LibraryAction;
use crate::app::state::{PlaybackAction, SessionAction};
use crate::app::{AppModel, Dispatcher};
use riff_core::library;
use std::rc::Rc;

pub struct UserMenuModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
}

impl UserMenuModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    pub fn logout(&self) {
        self.dispatcher.dispatch(PlaybackAction::Stop.into());
        let api = self.app_model.api();
        let dispatcher = self.dispatcher.clone();
        tokio::spawn(async move {
            api.clear_user_cache().await;
            dispatcher.dispatch(SessionAction::Logout.into());
        });
    }

    pub fn fetch_user_playlists(&self) {
        let api = self.app_model.api();
        dispatch_api_read(&self.dispatcher, move |tag| async move {
            library::load_saved_playlists(&api, tag)
                .await
                .map(|playlists| LibraryAction::PlaylistsLoaded(playlists).into())
        });
    }
}
