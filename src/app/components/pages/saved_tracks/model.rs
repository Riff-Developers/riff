use gettextrs::{gettext, ngettext};
use std::ops::Deref;
use std::rc::Rc;

use crate::{impl_source_page, impl_source_track_list, impl_track_list_model_base};

use crate::app::components::{labels, DetailsPageModel};
use crate::app::components::{
    HasHeaderBarModel, HeaderImageShape, PageModel, PinnedPageModel, ScreenModel, TrackListModel,
};
use crate::app::models::*;
use crate::app::state::{HomePage, PageFields, Route, SelectionContext, SelectionState};
use crate::app::{AppEvent, AppModel, Dispatcher, SongsSource};
use riff_core::screens::SavedTracksScreen;

const TRACKS_PAGE: usize = 50;

pub struct SavedTracksModel {
    base: DetailsPageModel,
    screen: ScreenModel<SavedTracksScreen>,
    songs: SongListModel,
}

impl Deref for SavedTracksModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl HasHeaderBarModel for SavedTracksModel {}

impl PinnedPageModel for SavedTracksModel {}

impl SavedTracksModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let base = DetailsPageModel::new_without_id(Rc::clone(&app_model), dispatcher.clone());
        let songs = SongListModel::new(TRACKS_PAGE as u32);
        let rows = songs.clone();
        let screen = ScreenModel::new(
            SavedTracksScreen::new(base.liked.clone()),
            &app_model,
            dispatcher,
            move |screen, changed| {
                if changed.contains(PageFields::TRACKS) {
                    rows.clone().sync_pages(&screen.tracks);
                }
            },
        );
        Self {
            base,
            screen,
            songs,
        }
    }

    /// Called on login to load the initial batch of saved tracks.
    pub fn load_initial(&self) {
        self.screen.open();
    }
}

impl PageModel for SavedTracksModel {
    fn get_title(&self) -> Option<String> {
        Some(gettext("All Tracks"))
    }

    fn get_subtitle(&self) -> Option<String> {
        let count = self.screen.get().count();
        let text = ngettext("{} Track", "{} Tracks", count as u32);
        Some(labels::fill(text, &[&count.to_string()]))
    }

    fn header_image_shape(&self) -> HeaderImageShape {
        HeaderImageShape::Square
    }

    fn songs_source(&self) -> Option<SongsSource> {
        Some(SongsSource::SavedTracks)
    }

    fn context_name(&self) -> Option<String> {
        Some(crate::app::components::labels::SAVED_TRACKS.clone())
    }

    fn default_icon(&self) -> Option<&str> {
        Some("emote-love-symbolic")
    }

    fn load_more(&self) {
        self.screen.load_more();
    }

    fn apply_event(&self, event: &AppEvent) {
        self.screen.apply(event);
    }

    fn is_loaded(&self) -> bool {
        true
    }

    impl_source_page!();

    fn should_refresh_details(&self, event: &AppEvent) -> bool {
        event
            .page_loaded(&Route::Home(HomePage::SavedTracks))
            .is_some()
    }
}

impl TrackListModel for SavedTracksModel {
    fn song_list_model(&self) -> SongListModel {
        self.songs.clone()
    }

    fn autoscroll_to_playing(&self) -> bool {
        true
    }

    fn show_album_column(&self) -> bool {
        true
    }

    impl_track_list_model_base!();
    impl_source_track_list!();

    fn enable_selection(&self) -> bool {
        self.enable_selection_with_context(SelectionContext::SavedTracks)
    }
}

crate::impl_track_list_header_bar!(SavedTracksModel);

impl crate::app::ProvidesApi for SavedTracksModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
