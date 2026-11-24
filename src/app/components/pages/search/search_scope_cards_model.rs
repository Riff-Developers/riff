use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::{sync_cards, CardListModel, ImageShape, ScreenModel};
use crate::app::models::*;
use crate::app::{AppAction, AppModel, Dispatcher, ListStore};

use riff_core::screens::SearchScreen;

use super::load_more_scope;

pub struct SearchScopeCardsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    screen: ScreenModel<SearchScreen>,
    cards: ListStore<CardModel>,
}

impl SearchScopeCardsModel {
    pub fn new(
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
        screen: ScreenModel<SearchScreen>,
    ) -> Self {
        Self {
            app_model,
            dispatcher,
            screen,
            cards: ListStore::new(),
        }
    }

    /// Shows the filtered results of the search.
    pub fn sync(&self) {
        let screen = self.screen.get();
        let scope = &screen.scope;
        let cards = match self.filter() {
            Some(SearchType::Album) => scope.albums.items.iter().map(CardModel::from).collect(),
            Some(SearchType::Artist) => scope.artists.items.iter().map(CardModel::from).collect(),
            Some(SearchType::Playlist) => {
                scope.playlists.items.iter().map(CardModel::from).collect()
            }
            _ => vec![],
        };
        sync_cards(&self.cards, cards);
    }

    fn filter(&self) -> Option<SearchType> {
        self.app_model.get_state().browser.search_filter()
    }
}

impl CardListModel for SearchScopeCardsModel {
    fn get_store(&self) -> Option<impl Deref<Target = ListStore<CardModel>> + '_> {
        Some(&self.cards)
    }

    fn refresh(&self) {}

    fn has_items(&self) -> bool {
        self.get_store().map(|s| s.len() > 0).unwrap_or(false)
    }

    fn has_more(&self) -> bool {
        self.screen.get().has_more()
    }

    fn load_more(&self) {
        let Some(search_type) = self.filter() else {
            return;
        };
        if search_type == SearchType::Track {
            return;
        }
        load_more_scope(&self.app_model, &self.screen, search_type);
    }

    fn open_item(&self, id: String) {
        match self.filter() {
            Some(SearchType::Album) => self.dispatcher.dispatch(AppAction::ViewAlbum(id)),
            Some(SearchType::Artist) => self.dispatcher.dispatch(AppAction::ViewArtist(id)),
            Some(SearchType::Playlist) => self.dispatcher.dispatch(AppAction::ViewPlaylist(id)),
            _ => {}
        }
    }

    fn image_shape(&self) -> ImageShape {
        match self.filter() {
            Some(SearchType::Artist) => ImageShape::Round,
            _ => ImageShape::Square,
        }
    }
}

impl crate::app::ProvidesApi for SearchScopeCardsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
