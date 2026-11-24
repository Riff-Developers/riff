use std::cell::Cell;
use std::ops::Deref;
use std::rc::Rc;

use gettextrs::gettext;

use crate::app::components::sync_cards;
use crate::app::components::{
    playing_card, CardLayout, CardListComponent, CardListModel, CardListPageModel, CardSize,
    ImageShape, PlayingCard, ScreenModel, SortOrder,
};
use crate::app::models::*;
use crate::app::state::{HomePage, Route};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, ListStore};
use riff_core::screens::SavedAlbumsScreen;

pub struct SavedAlbumsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    screen: ScreenModel<SavedAlbumsScreen>,
    cards: Rc<ListStore<CardModel>>,
}

impl SavedAlbumsModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let cards = Rc::new(ListStore::new());
        let store = Rc::clone(&cards);
        let screen = ScreenModel::new(
            SavedAlbumsScreen::default(),
            &app_model,
            dispatcher.clone(),
            move |screen, _| {
                let albums = &screen.albums.items;
                sync_cards(&store, albums.iter().map(CardModel::from).collect());
            },
        );
        Self {
            app_model,
            dispatcher,
            screen,
            cards,
        }
    }
}

impl CardListModel for SavedAlbumsModel {
    fn playing_card(&self) -> PlayingCard {
        playing_card(&self.app_model.get_state().playback)
    }

    fn get_store(&self) -> Option<impl Deref<Target = ListStore<CardModel>> + '_> {
        Some(self.cards.as_ref())
    }

    fn refresh(&self) {
        self.screen.open();
    }

    fn has_items(&self) -> bool {
        !self.cards.is_empty()
    }

    fn has_more(&self) -> bool {
        self.screen.get().albums.has_more()
    }

    fn load_more(&self) {
        self.screen.load_more();
    }

    fn open_item(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewAlbum(id));
    }

    fn image_shape(&self) -> ImageShape {
        ImageShape::Square
    }

    fn filter_options(&self) -> Vec<FilterOption> {
        FilterOption::release_types()
    }
}

impl CardListPageModel for SavedAlbumsModel {
    fn page_id(&self) -> &str {
        "albums"
    }
    fn empty_title(&self) -> String {
        gettext("You have no saved albums.")
    }
    fn empty_description(&self) -> String {
        gettext("Your library will be shown here.")
    }

    fn available_sort_orders(&self) -> &[SortOrder] {
        &[
            SortOrder::RecentlyAdded,
            SortOrder::DateReleased,
            SortOrder::Alphabetic,
            SortOrder::Creator,
            SortOrder::Popularity,
        ]
    }

    fn apply_event(&self, event: &AppEvent) {
        self.screen.apply(event);
    }

    fn should_refresh(&self, event: &AppEvent) -> bool {
        event.page_loaded(&Route::Home(HomePage::Library)).is_some()
    }
}

pub type SavedAlbums = CardListComponent<SavedAlbumsModel>;

pub fn make_saved_albums(
    model: SavedAlbumsModel,
    shared_layout: Rc<Cell<CardLayout>>,
    shared_size: Rc<Cell<CardSize>>,
    dispatcher: Dispatcher,
) -> SavedAlbums {
    CardListComponent::new(Rc::new(model), shared_layout, shared_size, dispatcher)
}

impl crate::app::ProvidesApi for SavedAlbumsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
