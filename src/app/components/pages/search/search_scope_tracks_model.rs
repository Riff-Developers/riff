use gio::SimpleActionGroup;
use std::ops::Deref;
use std::rc::Rc;

use crate::impl_track_list_model_base;

use crate::app::components::DetailsPageModel;
use crate::app::components::{source_play_context, track_menu, ScreenModel, TrackListModel};
use crate::app::models::*;
use crate::app::state::{LikedTracks, Route, SelectionState, PAGE_SIZE};
use crate::app::state::{PlayContext, SelectionContext};
use crate::app::{AppEvent, AppModel, Dispatcher, SongsSource};

use riff_core::screens::SearchScreen;

use super::load_more_scope;

pub struct SearchScopeTracksModel {
    base: DetailsPageModel,
    screen: ScreenModel<SearchScreen>,
    songs: SongListModel,
}

impl Deref for SearchScopeTracksModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl SearchScopeTracksModel {
    pub fn new(
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
        screen: ScreenModel<SearchScreen>,
        liked: LikedTracks,
    ) -> Self {
        let mut base = DetailsPageModel::new_without_id(app_model, dispatcher);
        base.liked = liked;
        Self {
            base,
            screen,
            songs: SongListModel::new(PAGE_SIZE as u32),
        }
    }

    fn sync_songs(&self) {
        let screen = self.screen.get();
        let tracks = &screen.scope.tracks.items;
        self.songs.clone().sync(tracks, None, screen.is_complete());
    }

    pub fn get_query(&self) -> Option<String> {
        Some(
            self.app_model
                .get_state()
                .browser
                .search_query()
                .to_string(),
        )
    }

    pub fn load_more(&self) {
        load_more_scope(&self.app_model, &self.screen, SearchType::Track);
    }
}

impl TrackListModel for SearchScopeTracksModel {
    fn apply_event(&self, event: &AppEvent) {
        if event.search().is_some() || event.page_loaded(&Route::Search).is_some() {
            self.sync_songs();
        }
    }

    fn song_list_model(&self) -> SongListModel {
        self.songs.clone()
    }

    fn autoscroll_to_playing(&self) -> bool {
        false
    }

    fn show_album_column(&self) -> bool {
        true
    }

    fn load_more(&self) {
        SearchScopeTracksModel::load_more(self);
    }

    impl_track_list_model_base!();

    fn enable_selection(&self) -> bool {
        self.enable_selection_with_context(SelectionContext::Default)
    }

    fn play_song_at(&self, _pos: usize, id: &str) {
        self.play_in_context(TrackListModel::play_context(self), id);
    }

    fn play_context(&self) -> Option<PlayContext> {
        let query = self.get_query().unwrap_or_default();
        let songs = TrackListModel::song_list_model(self);
        Some(source_play_context(
            SongsSource::Search(query.clone()),
            Some(query),
            &songs,
        ))
    }

    fn actions_for(&self, _row: &SongModel, song: &Track) -> Option<SimpleActionGroup> {
        Some(self.base.track_actions(song, None))
    }

    fn playlist_picker(&self, song: &Track) -> Option<gtk::Widget> {
        self.base.playlist_picker(song)
    }

    fn menu_for(
        &self,
        _row: &SongModel,
        song: &Track,
        liked: bool,
        pinned: Option<bool>,
    ) -> Option<gio::MenuModel> {
        Some(track_menu(song, None, liked, pinned))
    }
}

impl crate::app::ProvidesApi for SearchScopeTracksModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
