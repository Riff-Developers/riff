use gettextrs::gettext;
use std::cell::Ref;
use std::ops::Deref;
use std::rc::Rc;
use tokio::sync::oneshot;

use crate::app::components::DetailsPageModel;
use crate::app::components::{
    dispatch_api_call, dispatch_api_call_many, labels, upload_playlist_cover, HasHeaderBarModel,
    HeaderImageShape, PageModel, PinnedPageModel, PlaylistForm, Saved, ScreenModel, TrackListModel,
};
use crate::app::models::*;
use crate::app::state::{
    ItemId, NavigationAction, PageFields, Route, SelectionContext, SelectionState,
};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher, SongsSource};
use crate::{impl_source_page, impl_source_track_list, impl_track_list_model_base};
use riff_core::library;
use riff_core::screens::PlaylistScreen;

const TRACKS_PAGE: usize = 50;

pub struct PlaylistDetailsModel {
    base: DetailsPageModel,
    screen: ScreenModel<PlaylistScreen>,
    songs: SongListModel,
}

impl Deref for PlaylistDetailsModel {
    type Target = DetailsPageModel;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl HasHeaderBarModel for PlaylistDetailsModel {}

impl PlaylistDetailsModel {
    pub fn new(id: String, app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        let base = DetailsPageModel::new(id.clone(), Rc::clone(&app_model), dispatcher.clone());
        let songs = SongListModel::new(TRACKS_PAGE as u32);
        let rows = songs.clone();
        let screen = ScreenModel::new(
            PlaylistScreen::new(id, base.liked.clone()),
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

    fn route(&self) -> Route {
        Route::Playlist(self.id.clone())
    }

    /// Returns true if the logged-in user owns this playlist.
    pub fn is_playlist_editable(&self) -> bool {
        self.owns_playlist(&self.id)
    }

    pub fn get_playlist_info(&self) -> Option<impl Deref<Target = Playlist> + '_> {
        Ref::filter_map(self.screen.get(), |s| s.playlist.as_ref()).ok()
    }

    /// Rename the playlist via the API and update local state.
    pub fn update_playlist_details(&self, title: String) {
        let api = self.app_model.api();
        let id = self.id.clone();
        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(library::rename_playlist(&api, id, title).await?.into())
        });
    }

    pub fn edit_playlist(&self, form: PlaylistForm) -> oneshot::Receiver<()> {
        let (saved, saved_rx) = Saved::channel();
        let api = self.app_model.api();
        let id = self.id.clone();
        let dispatcher = self.dispatcher.clone();
        let details = form.changed_details();
        let current = self.get_playlist_info().and_then(|p| p.description.clone());
        let kept = details.keeps_description(current.as_deref());
        dispatch_api_call_many(&self.dispatcher, move || async move {
            let edited = library::edit_playlist(&api, id.clone(), details).await?;
            saved.tell();
            if let Some(cover) = form.cover {
                upload_playlist_cover(&dispatcher, api, id, cover);
            }
            let mut actions: Vec<AppAction> = edited.into_iter().map(AppAction::from).collect();
            if kept {
                actions.push(AppAction::ShowNotification(gettext(
                    // translators: This notification is shown when the user emptied the description of a playlist, which cannot be removed once set.
                    "A playlist's description can't be removed, so it was kept.",
                )));
            }
            Ok(actions)
        });
        saved_rx
    }

    pub fn delete_playlist(&self) {
        let api = self.app_model.api();
        let id = self.id.clone();
        dispatch_api_call_many(&self.dispatcher, move || async move {
            let leave = NavigationAction::Leave(Route::Playlist(id.clone())).into();
            let removed = library::set_saved(&api, ItemId::Playlist(id), false).await?;
            Ok(vec![removed.into(), leave])
        });
    }
}

impl PageModel for PlaylistDetailsModel {
    fn get_title(&self) -> Option<String> {
        Some(self.get_playlist_info()?.title.clone())
    }

    fn get_subtitle(&self) -> Option<String> {
        Some(
            self.get_playlist_info()?
                .owner
                .as_ref()
                .map(|o| o.display_name.clone())
                .unwrap_or_default(),
        )
    }

    fn get_subtitle_detail(&self) -> Option<String> {
        let count = self.get_playlist_info()?.total_tracks?;
        let text = gettextrs::ngettext("{} Track", "{} Tracks", count);
        Some(labels::fill(text, &[&count.to_string()]))
    }

    fn get_artwork(&self) -> Option<ImageSet> {
        Some(self.get_playlist_info()?.art.clone())
    }

    fn get_caption(&self) -> Option<String> {
        let public = self.get_playlist_info().and_then(|p| p.public);
        Some(match public {
            Some(true) => labels::PUBLIC_PLAYLIST_CAPTION.clone(),
            Some(false) => labels::PRIVATE_PLAYLIST_CAPTION.clone(),
            None => gettext("Playlist"),
        })
    }

    fn header_image_shape(&self) -> HeaderImageShape {
        HeaderImageShape::Square
    }

    fn songs_source(&self) -> Option<SongsSource> {
        Some(SongsSource::Playlist(self.id.clone()))
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
        self.get_playlist_info().is_some()
    }

    impl_source_page!();

    fn has_like_button(&self) -> bool {
        true
    }

    fn like_tooltip(&self, is_liked: bool) -> Option<String> {
        Some(if is_liked {
            labels::UNLIKE_PLAYLIST.clone()
        } else {
            labels::LIKE_PLAYLIST.clone()
        })
    }

    fn is_liked(&self) -> bool {
        self.app_model.get_state().library.has_playlist(&self.id)
    }

    fn toggle_like(&self) {
        let id = self.id.clone();
        let is_saved = self.is_liked();
        let api = self.app_model.api();
        let description = self.get_playlist_info().map(|p| p.clone());

        dispatch_api_call(&self.dispatcher, move || async move {
            if is_saved {
                Ok(library::set_saved(&api, ItemId::Playlist(id), false)
                    .await?
                    .into())
            } else {
                let playlist = description.unwrap_or_else(|| Playlist {
                    rri: ResourceId {
                        id: id.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                Ok(library::follow_playlist(&api, playlist).await?.into())
            }
        });
    }

    fn like_visible(&self) -> bool {
        !self.is_playlist_editable()
    }

    fn get_subtitle_links(&self) -> Vec<ArtistRef> {
        self.get_playlist_info()
            .and_then(|playlist| {
                playlist.owner.as_ref().map(|o| {
                    vec![ArtistRef {
                        rri: o.rri.clone(),
                        name: o.display_name.clone(),
                    }]
                })
            })
            .unwrap_or_default()
    }

    fn navigate_to_subtitle_link(&self, id: &str) {
        self.dispatcher
            .dispatch(AppAction::ViewUser(id.to_string()));
    }

    fn should_refresh_details(&self, event: &AppEvent) -> bool {
        event
            .page_loaded(&self.route())
            .is_some_and(|loaded| loaded.contains(PageFields::INFO))
    }

    fn should_refresh_liked(&self, event: &AppEvent) -> bool {
        event.saved(&ItemId::Playlist(self.id.clone())).is_some()
    }
}

impl PinnedPageModel for PlaylistDetailsModel {
    fn supports_pin_button(&self) -> bool {
        true
    }
}

impl TrackListModel for PlaylistDetailsModel {
    fn song_list_model(&self) -> SongListModel {
        self.songs.clone()
    }

    fn show_album_column(&self) -> bool {
        true
    }

    impl_track_list_model_base!();
    impl_source_track_list!();

    fn enable_selection(&self) -> bool {
        let context = if self.is_playlist_editable() {
            SelectionContext::EditablePlaylist(self.id.clone())
        } else {
            SelectionContext::Playlist
        };
        self.enable_selection_with_context(context)
    }
}

crate::impl_track_list_header_bar!(PlaylistDetailsModel);

impl crate::app::ProvidesApi for PlaylistDetailsModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }
}
