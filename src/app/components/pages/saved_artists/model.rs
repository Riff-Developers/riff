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
use riff_core::screens::SavedArtistsScreen;

pub struct SavedArtistsModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    screen: ScreenModel<SavedArtistsScreen>,
    cards: Rc<ListStore<CardModel>>,
}

impl SavedArtistsModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let cards = Rc::new(ListStore::new());
        let store = Rc::clone(&cards);
        let screen = ScreenModel::new(
            SavedArtistsScreen::default(),
            &app_model,
            dispatcher.clone(),
            move |screen, _| {
                let artists = &screen.artists;
                sync_cards(&store, artists.iter().map(CardModel::from).collect());
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

impl CardListModel for SavedArtistsModel {
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
        self.screen.get().has_more()
    }

    fn load_more(&self) {
        self.screen.load_more();
    }

    fn open_item(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewArtist(id));
    }

    fn image_shape(&self) -> ImageShape {
        ImageShape::Round
    }
}

impl CardListPageModel for SavedArtistsModel {
    fn page_id(&self) -> &str {
        "artists"
    }
    fn empty_title(&self) -> String {
        gettext("You have no saved artists.")
    }
    fn empty_description(&self) -> String {
        gettext("Your followed artists will be shown here.")
    }
    fn empty_icon(&self) -> &str {
        "avatar-default-symbolic"
    }

    fn available_sort_orders(&self) -> &[SortOrder] {
        &[
            SortOrder::RecentlyAdded,
            SortOrder::Alphabetic,
            SortOrder::Popularity,
        ]
    }

    fn apply_event(&self, event: &AppEvent) {
        self.screen.apply(event);
    }

    fn should_refresh(&self, event: &AppEvent) -> bool {
        event
            .page_loaded(&Route::Home(HomePage::SavedArtists))
            .is_some()
    }
}

pub type SavedArtists = CardListComponent<SavedArtistsModel>;

pub fn make_saved_artists(
    model: SavedArtistsModel,
    shared_layout: Rc<Cell<CardLayout>>,
    shared_size: Rc<Cell<CardSize>>,
    dispatcher: Dispatcher,
) -> SavedArtists {
    CardListComponent::new(Rc::new(model), shared_layout, shared_size, dispatcher)
}

impl crate::app::ProvidesApi for SavedArtistsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
