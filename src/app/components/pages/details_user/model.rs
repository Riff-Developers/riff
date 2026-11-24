use gettextrs::gettext;
use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::sync_cards;
use crate::app::components::DetailsPageModel;
use crate::app::components::{
    playing_card, CardListModel, HasHeaderBarModel, HeaderImageShape, ImageShape, PageModel,
    PinnedPageModel, PlayingCard, ScreenModel, SimpleHeaderBarModel,
};
use crate::app::models::*;
use crate::app::state::{PageFields, Route};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, ListStore};
use riff_core::screens::UserScreen;

pub struct UserDetailsModel {
    base: DetailsPageModel,
    screen: ScreenModel<UserScreen>,
    playlists: Rc<ListStore<CardModel>>,
}

impl Deref for UserDetailsModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl HasHeaderBarModel for UserDetailsModel {}

impl PinnedPageModel for UserDetailsModel {}

impl UserDetailsModel {
    pub fn new(id: String, app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let base = DetailsPageModel::new(id.clone(), Rc::clone(&app_model), dispatcher.clone());
        let playlists = Rc::new(ListStore::new());
        let cards = Rc::clone(&playlists);
        let screen = ScreenModel::new(
            UserScreen::new(id),
            &app_model,
            dispatcher,
            move |screen, changed| {
                if changed.contains(PageFields::CARDS) {
                    let playlists = &screen.playlists.items;
                    sync_cards(&cards, playlists.iter().map(CardModel::from).collect());
                }
            },
        );
        Self {
            base,
            screen,
            playlists,
        }
    }

    fn route(&self) -> Route {
        Route::User(self.id.clone())
    }
}

impl PageModel for UserDetailsModel {
    fn apply_event(&self, event: &AppEvent) {
        self.screen.apply(event);
    }

    fn get_title(&self) -> Option<String> {
        Some(self.screen.get().user.as_ref()?.display_name.clone())
    }

    fn get_caption(&self) -> Option<String> {
        Some(gettext("Profile"))
    }

    fn get_artwork(&self) -> Option<ImageSet> {
        Some(self.screen.get().user.as_ref()?.art.clone())
    }

    fn header_image_shape(&self) -> HeaderImageShape {
        HeaderImageShape::Circle
    }

    fn load_page_info(&self) {
        self.screen.open();
    }

    fn load_more(&self) {
        self.screen.load_more();
    }

    fn is_loaded(&self) -> bool {
        self.screen.get().user.is_some()
    }

    fn should_refresh_details(&self, event: &AppEvent) -> bool {
        event.page_loaded(&self.route()).is_some()
    }

    fn has_share_button(&self) -> bool {
        true
    }

    fn on_share_clicked(&self) {
        self.share_link(&format!("https://open.spotify.com/user/{}", self.id));
    }
}

impl CardListModel for UserDetailsModel {
    fn playing_card(&self) -> PlayingCard {
        playing_card(&self.state().playback)
    }

    fn get_store(&self) -> Option<impl Deref<Target = ListStore<CardModel>> + '_> {
        Some(self.playlists.as_ref())
    }

    fn load_more(&self) {
        PageModel::load_more(self);
    }

    fn refresh(&self) {}

    fn has_items(&self) -> bool {
        !self.playlists.is_empty()
    }

    fn open_item(&self, id: String) {
        self.dispatcher.dispatch(AppAction::ViewPlaylist(id));
    }

    fn image_shape(&self) -> ImageShape {
        ImageShape::Square
    }
}

impl SimpleHeaderBarModel for UserDetailsModel {
    fn select_all(&self) {}
}

impl crate::app::ProvidesApi for UserDetailsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
