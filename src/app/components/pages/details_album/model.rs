use std::cell::Ref;
use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::DetailsPageModel;
use crate::app::components::{
    dispatch_api_call, labels, HasHeaderBarModel, HeaderImageShape, PageModel, PinnedPageModel,
    ScreenModel, TrackListModel,
};
use crate::app::models::*;
use crate::app::state::PinKind;
use crate::app::state::{ItemId, PageFields, Route, SelectionContext, SelectionState, PAGE_SIZE};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, SongsSource};
use crate::{impl_source_page, impl_source_track_list, impl_track_list_model_base};
use riff_core::library;
use riff_core::screens::AlbumScreen;

pub struct DetailsModel {
    base: DetailsPageModel,
    screen: ScreenModel<AlbumScreen>,
    songs: SongListModel,
}

impl Deref for DetailsModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl HasHeaderBarModel for DetailsModel {}

impl DetailsModel {
    pub fn new(id: String, app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let base = DetailsPageModel::new(id.clone(), Rc::clone(&app_model), dispatcher.clone());
        let songs = SongListModel::new(PAGE_SIZE as u32);
        let rows = songs.clone();
        let screen = ScreenModel::new(
            AlbumScreen::new(id, base.liked.clone()),
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

    pub fn get_album_info(&self) -> Option<impl Deref<Target = Album> + '_> {
        Ref::filter_map(self.screen.get(), |s| s.album.as_ref()).ok()
    }
}

impl PageModel for DetailsModel {
    fn get_title(&self) -> Option<String> {
        Some(self.get_album_info()?.title.clone())
    }

    fn get_subtitle(&self) -> Option<String> {
        Some(self.get_album_info()?.artists_name())
    }

    fn get_artwork(&self) -> Option<ImageSet> {
        Some(self.get_album_info()?.art.clone())
    }

    fn get_caption(&self) -> Option<String> {
        let album = self.get_album_info()?;
        Some(match album.album_type {
            AlbumType::Single => labels::SINGLE_CAPTION.clone(),
            AlbumType::Ep => labels::EP_CAPTION.clone(),
            AlbumType::Compilation => labels::COMPILATION_CAPTION.clone(),
            _ => labels::ALBUM_CAPTION.clone(),
        })
    }

    fn header_image_shape(&self) -> HeaderImageShape {
        HeaderImageShape::Square
    }

    fn songs_source(&self) -> Option<SongsSource> {
        Some(SongsSource::Album(self.id.clone()))
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
        self.get_album_info().is_some()
    }

    impl_source_page!();

    fn has_like_button(&self) -> bool {
        true
    }

    fn like_tooltip(&self, is_liked: bool) -> Option<String> {
        Some(if is_liked {
            labels::UNLIKE_ALBUM.clone()
        } else {
            labels::LIKE_ALBUM.clone()
        })
    }

    fn is_liked(&self) -> bool {
        self.screen.get().saved
    }

    fn toggle_like(&self) {
        let album = {
            let Some(album) = self.get_album_info() else {
                return;
            };
            (*album).clone()
        };
        let id = album.rri.id.clone();
        let is_liked = self.is_liked();
        let api = self.app_model.api();
        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(library::set_saved(&api, ItemId::Album(id), !is_liked)
                .await?
                .into())
        });
    }

    fn get_subtitle_links(&self) -> Vec<ArtistRef> {
        self.get_album_info()
            .map(|album| album.artists.clone())
            .unwrap_or_default()
    }

    fn navigate_to_subtitle_link(&self, id: &str) {
        self.dispatcher
            .dispatch(AppAction::ViewArtist(id.to_string()));
    }

    fn should_refresh_details(&self, event: &AppEvent) -> bool {
        event
            .page_loaded(&Route::Album(self.id.clone()))
            .is_some_and(|loaded| loaded.contains(PageFields::INFO))
    }

    fn should_refresh_liked(&self, event: &AppEvent) -> bool {
        let route = Route::Album(self.id.clone());
        event
            .page_loaded(&route)
            .is_some_and(|loaded| loaded.contains(PageFields::SAVED))
            || event.saved(&ItemId::Album(self.id.clone())).is_some()
    }
}

impl PinnedPageModel for DetailsModel {
    fn pin_kind(&self) -> PinKind {
        PinKind::Album
    }

    fn supports_pin_button(&self) -> bool {
        true
    }
}

impl TrackListModel for DetailsModel {
    fn song_list_model(&self) -> SongListModel {
        self.songs.clone()
    }

    fn show_song_covers(&self) -> bool {
        false
    }

    fn show_disc_headers(&self) -> bool {
        true
    }

    impl_track_list_model_base!();
    impl_source_track_list!();

    fn enable_selection(&self) -> bool {
        self.enable_selection_with_context(SelectionContext::Default)
    }
}

crate::impl_track_list_header_bar!(DetailsModel);

impl crate::app::ProvidesApi for DetailsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
