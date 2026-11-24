use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::sync_cards;
use crate::app::components::DetailsPageModel;
use crate::app::components::{
    dispatch_api_call, labels, playing_card, CardListModel, HasHeaderBarModel, HeaderImageShape,
    ImageShape, PageModel, PinnedPageModel, PlayingCard, ScreenModel, TrackListModel,
};
use crate::app::models::*;
use crate::app::state::PinKind;
use crate::app::state::{ItemId, PageFields, Route, SelectionContext, SelectionState};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, ListStore, SongsSource};
use crate::{impl_source_page, impl_source_track_list, impl_track_list_model_base};
use riff_core::library;
use riff_core::screens::ArtistScreen;

pub struct ArtistDetailsModel {
    base: DetailsPageModel,
    screen: ScreenModel<ArtistScreen>,
    releases: Rc<ListStore<CardModel>>,
    top_tracks: SongListModel,
}

impl Deref for ArtistDetailsModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl HasHeaderBarModel for ArtistDetailsModel {}

impl ArtistDetailsModel {
    pub fn new(id: String, app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let base = DetailsPageModel::new(id.clone(), Rc::clone(&app_model), dispatcher.clone());
        let releases = Rc::new(ListStore::new());
        let top_tracks = SongListModel::new(10);
        let (cards, rows) = (Rc::clone(&releases), top_tracks.clone());
        let screen = ScreenModel::new(
            ArtistScreen::new(id, base.liked.clone()),
            &app_model,
            dispatcher,
            move |screen, changed| {
                if changed.contains(PageFields::CARDS) {
                    let releases = &screen.releases.items;
                    sync_cards(&cards, releases.iter().map(CardModel::from).collect());
                }
                if changed.contains(PageFields::TRACKS) {
                    rows.clone().sync(&screen.top_tracks, None, true);
                }
            },
        );
        Self {
            base,
            screen,
            releases,
            top_tracks,
        }
    }

    fn route(&self) -> Route {
        Route::Artist(self.id.clone())
    }
}

impl PageModel for ArtistDetailsModel {
    fn get_title(&self) -> Option<String> {
        Some(self.screen.get().artist.as_ref()?.name.clone())
    }

    fn get_artwork(&self) -> Option<ImageSet> {
        Some(self.screen.get().artist.as_ref()?.art.clone())
    }

    fn get_caption(&self) -> Option<String> {
        Some("Artist".to_string())
    }

    fn header_image_shape(&self) -> HeaderImageShape {
        HeaderImageShape::Circle
    }

    fn songs_source(&self) -> Option<SongsSource> {
        Some(SongsSource::Artist(self.id.clone()))
    }

    fn load_page_info(&self) {
        self.screen.open();
    }

    fn load_more(&self) {
        self.screen.load_more();
    }

    fn apply_event(&self, event: &AppEvent) {
        self.screen.apply(event);
    }

    fn is_loaded(&self) -> bool {
        self.screen.get().artist.is_some()
    }

    impl_source_page!();

    fn has_like_button(&self) -> bool {
        true
    }

    fn like_tooltip(&self, is_liked: bool) -> Option<String> {
        Some(if is_liked {
            labels::UNLIKE_ARTIST.clone()
        } else {
            labels::LIKE_ARTIST.clone()
        })
    }

    fn is_liked(&self) -> bool {
        self.screen.get().followed
    }

    fn toggle_like(&self) {
        let id = self.id.clone();
        let is_followed = self.is_liked();
        let api = self.app_model.api();

        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(library::set_saved(&api, ItemId::Artist(id), !is_followed)
                .await?
                .into())
        });
    }

    fn should_refresh_details(&self, event: &AppEvent) -> bool {
        event.page_loaded(&self.route()).is_some()
    }

    fn should_refresh_liked(&self, event: &AppEvent) -> bool {
        event
            .page_loaded(&self.route())
            .is_some_and(|loaded| loaded.contains(PageFields::SAVED))
            || event.saved(&ItemId::Artist(self.id.clone())).is_some()
    }
}

impl PinnedPageModel for ArtistDetailsModel {
    fn pin_kind(&self) -> PinKind {
        PinKind::Artist
    }

    fn supports_pin_button(&self) -> bool {
        true
    }
}

impl CardListModel for ArtistDetailsModel {
    fn playing_card(&self) -> PlayingCard {
        playing_card(&self.state().playback)
    }

    fn get_store(&self) -> Option<impl Deref<Target = ListStore<CardModel>> + '_> {
        Some(self.releases.as_ref())
    }

    fn load_more(&self) {
        PageModel::load_more(self);
    }
    fn refresh(&self) {}

    fn has_items(&self) -> bool {
        !self.releases.is_empty()
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

impl TrackListModel for ArtistDetailsModel {
    fn song_list_model(&self) -> SongListModel {
        self.top_tracks.clone()
    }

    fn show_album_column(&self) -> bool {
        true
    }

    impl_track_list_model_base!();
    impl_source_track_list!();

    fn enable_selection(&self) -> bool {
        self.enable_selection_with_context(SelectionContext::Default)
    }
}

crate::impl_track_list_header_bar!(ArtistDetailsModel);

impl crate::app::ProvidesApi for ArtistDetailsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
