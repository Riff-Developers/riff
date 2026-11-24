use std::cell::{Ref, RefCell};
use std::rc::Rc;

use crate::app::components::ScreenModel;
use crate::app::dispatch::Dispatcher;
use crate::app::models::*;
use crate::app::state::{AppAction, AppModel, LikedTracks, SearchAction};
use riff_core::screens::SearchScreen;

pub struct SearchResultsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    screen: ScreenModel<SearchScreen>,
    liked: LikedTracks,
    queued_song: RefCell<Option<Track>>,
}

impl SearchResultsModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let liked = LikedTracks::default();
        let screen = ScreenModel::new(
            SearchScreen::new(liked.clone()),
            &app_model,
            dispatcher.clone(),
            |_, _| {},
        );
        Self {
            queued_song: Default::default(),
            screen,
            liked,
            app_model,
            dispatcher,
        }
    }

    /// Shared app model, used to build the scoped view adapters.
    pub fn app_model(&self) -> Rc<AppModel> {
        Rc::clone(&self.app_model)
    }

    /// The loaded results, shared with the scoped view adapters.
    pub fn screen(&self) -> ScreenModel<SearchScreen> {
        self.screen.clone()
    }

    /// Whether the results' tracks are liked, shared with the scoped track view.
    pub fn liked(&self) -> LikedTracks {
        self.liked.clone()
    }

    pub fn search(&self, query: String) {
        self.dispatcher.dispatch(SearchAction::Query(query).into());
    }

    /// Set the active filter (`None` = combined results). Re-runs the search.
    pub fn set_filter(&self, filter: Option<SearchType>) {
        self.dispatcher
            .dispatch(SearchAction::Filter(filter).into());
    }

    pub fn get_filter(&self) -> Option<SearchType> {
        self.app_model.get_state().browser.search_filter()
    }

    fn get_query(&self) -> Option<String> {
        let query = self
            .app_model
            .get_state()
            .browser
            .search_query()
            .to_string();
        Some(query).filter(|q| !q.is_empty())
    }

    pub fn has_query(&self) -> bool {
        self.get_query().is_some()
    }

    /// Run the appropriate search for the current query and filter.
    pub fn fetch_results(&self) {
        let (query, filter) = {
            let state = self.app_model.get_state();
            let browser = &state.browser;
            (browser.search_query().to_string(), browser.search_filter())
        };
        self.screen.update(|screen| screen.search(&query, filter));
    }

    pub fn get_results(&self) -> Option<Ref<'_, SearchResults>> {
        Some(Ref::map(self.screen.get(), |s| &s.results))
    }

    pub fn open_track(&self, song: Track) {
        self.queued_song.borrow_mut().replace(song.clone());
        self.dispatcher.dispatch(AppAction::ViewAlbum(
            song.album
                .as_ref()
                .map(|a| a.rri.id.clone())
                .unwrap_or_default(),
        ));
    }
    pub fn on_album_loaded(&self, id: &str) {
        if let Some(song) = self.queued_song.borrow_mut().take() {
            if song.album.as_ref().map(|a| a.rri.id == id).unwrap_or(false) {
                self.dispatcher
                    .dispatch(AppAction::PlaySong(song.rri.id.clone()))
            }
        }
    }

    pub fn open_album(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewAlbum(id));
    }

    pub fn open_artist(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewArtist(id));
    }

    pub fn open_playlist(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewPlaylist(id));
    }
}

/// Load the next page of a scoped search view. Shared by the scoped card and
/// track adapters. No-op when the filter no longer matches `kind`.
pub(super) fn load_more_scope(
    app_model: &AppModel,
    screen: &ScreenModel<SearchScreen>,
    kind: SearchType,
) {
    if app_model.get_state().browser.search_filter() == Some(kind) {
        screen.load_more();
    }
}
