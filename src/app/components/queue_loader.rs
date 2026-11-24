use crate::app::components::dispatch_api_read;
use crate::app::models::SongsSource;
use crate::app::state::Action;
use crate::app::{AppAction, AppModel, Dispatcher};
use riff_core::play_queue::playable_tracks;

pub fn queue_source_tracks(app_model: &AppModel, dispatcher: &Dispatcher, source: SongsSource) {
    let api = app_model.api();
    dispatch_api_read(dispatcher, move |tag| async move {
        let tracks = playable_tracks(&api, &source, tag).await?;
        Ok(AppAction::Core(Action::QueueTracks(tracks)))
    });
}
