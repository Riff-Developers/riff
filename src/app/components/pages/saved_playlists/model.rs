use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use gettextrs::gettext;

use crate::app::components::{
    dispatch_api_read, playing_card, sync_cards, CardFilter, CardLayout, CardListComponent,
    CardListModel, CardListPageModel, CardSize, ImageShape, PlayingCard, SortOrder,
};
use crate::app::models::*;
use crate::app::state::LibraryAction;
use crate::app::state::LibraryFields;
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, ListStore};
use riff_core::library;

const OWNED_FILTER: &str = "owned";
const FOLLOWED_FILTER: &str = "followed";

pub struct SavedPlaylistsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    cards: RefCell<ListStore<CardModel>>,
}

impl SavedPlaylistsModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
            cards: RefCell::new(ListStore::new()),
        }
    }

    fn sync_cards(&self) {
        let cards = self
            .app_model
            .get_state()
            .library
            .playlists()
            .iter()
            .map(CardModel::from)
            .collect();
        sync_cards(&self.cards.borrow(), cards);
    }
}

impl CardListModel for SavedPlaylistsModel {
    fn playing_card(&self) -> PlayingCard {
        playing_card(&self.app_model.get_state().playback)
    }

    fn get_store(&self) -> Option<impl Deref<Target = ListStore<CardModel>> + '_> {
        Some(self.cards.borrow())
    }

    fn refresh(&self) {
        let api = self.app_model.api();
        dispatch_api_read(&self.dispatcher, move |tag| async move {
            library::load_saved_playlists(&api, tag)
                .await
                .map(|playlists| LibraryAction::PlaylistsLoaded(playlists).into())
        });
    }

    fn has_items(&self) -> bool {
        !self.cards.borrow().is_empty()
    }

    fn load_more(&self) {}

    fn open_item(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewPlaylist(id));
    }

    fn image_shape(&self) -> ImageShape {
        ImageShape::Square
    }

    fn filter_options(&self) -> Vec<FilterOption> {
        vec![
            // translators: Filter of the saved playlists: shows all of them.
            FilterOption::all(gettext("All")),
            // translators: Filter of the saved playlists: shows only those the user created.
            FilterOption::new(gettext("Created"), OWNED_FILTER),
            // translators: Filter of the saved playlists: shows only those made by others, which the user follows.
            FilterOption::new(gettext("Followed"), FOLLOWED_FILTER),
        ]
    }

    fn resolve_filter(&self, category: &str) -> Option<CardFilter> {
        let user = self.app_model.get_state().session.user.clone()?;
        match category {
            OWNED_FILTER => Some(CardFilter::OwnedBy(user)),
            FOLLOWED_FILTER => Some(CardFilter::NotOwnedBy(user)),
            _ => None,
        }
    }
}

impl CardListPageModel for SavedPlaylistsModel {
    fn page_id(&self) -> &str {
        "playlists"
    }
    fn empty_title(&self) -> String {
        gettext("You have no saved playlists.")
    }
    fn empty_description(&self) -> String {
        gettext("Your playlists will be shown here.")
    }

    fn available_sort_orders(&self) -> &[SortOrder] {
        &[
            SortOrder::RecentlyAdded,
            SortOrder::Alphabetic,
            SortOrder::Creator,
        ]
    }

    fn apply_event(&self, event: &AppEvent) {
        if event.library_changed(LibraryFields::PLAYLISTS) {
            self.sync_cards();
        }
    }

    fn should_refresh(&self, event: &AppEvent) -> bool {
        event.library_changed(LibraryFields::PLAYLISTS)
    }
}

pub type SavedPlaylists = CardListComponent<SavedPlaylistsModel>;

pub fn make_saved_playlists(
    model: SavedPlaylistsModel,
    shared_layout: Rc<Cell<CardLayout>>,
    shared_size: Rc<Cell<CardSize>>,
    dispatcher: Dispatcher,
) -> SavedPlaylists {
    CardListComponent::new(Rc::new(model), shared_layout, shared_size, dispatcher)
}

impl crate::app::ProvidesApi for SavedPlaylistsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
