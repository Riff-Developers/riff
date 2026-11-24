use gettextrs::gettext;
use gtk::prelude::*;
use std::rc::Rc;
use tokio::sync::oneshot;

use super::{
    NavigationPanelDestination, NavigationPanelItem, PINNED_SECTION, SAVED_PLAYLISTS_SECTION,
};
use crate::app::components::{
    dispatch_api_call, dispatch_api_call_many, dispatch_api_read, dispatch_api_read_many,
    queue_source_tracks, upload_playlist_cover, PlaylistForm, Saved,
};
use crate::app::models::{Playlist, PlaylistSummary};
use crate::app::state::PinKind;
use crate::app::state::{playback_actions, PlayContext, Route};
use crate::app::state::{ItemId, LibraryAction, NavigationAction, Pin};
use crate::app::{AppAction, AppModel, Dispatcher, SongsSource};
use riff_config::feature_flag::{is_enabled, FeatureFlag};
use riff_core::library;

pub struct NavigationPanelModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
}

impl NavigationPanelModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.app_model.get_state().playback.is_playing()
    }

    fn get_playlists(&self) -> Vec<NavigationPanelDestination> {
        self.app_model
            .get_state()
            .library
            .playlists()
            .iter()
            .map(Self::map_to_destination)
            .collect()
    }

    fn map_to_destination(playlist: &Playlist) -> NavigationPanelDestination {
        let title = Some(playlist.title.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| gettext("Unnamed Playlist"));
        let id = playlist.rri.id.clone();
        NavigationPanelDestination::Playlist(PlaylistSummary { id, title })
    }

    pub(super) fn create_new_playlist(&self, new: PlaylistForm) -> oneshot::Receiver<()> {
        let (created, created_rx) = Saved::channel();
        let user_id = self.app_model.get_state().session.user.clone().unwrap();
        let api = self.app_model.api();
        let dispatcher = self.dispatcher.clone();
        dispatch_api_call_many(&self.dispatcher, move || async move {
            let playlist = library::create_playlist(&api, &user_id, &new.new_details()).await?;
            created.tell();
            let id = playlist.rri.id.clone();
            if let Some(cover) = new.cover {
                upload_playlist_cover(&dispatcher, api, id.clone(), cover);
            }
            Ok(vec![
                AppAction::Core(LibraryAction::SavePlaylist(Box::new(playlist)).into()),
                AppAction::ViewPlaylist(id),
            ])
        });
        created_rx
    }

    pub(super) fn is_playlist_owned(&self, id: &str) -> bool {
        let state = self.app_model.get_state();
        let Some(user) = state.session.user.as_deref() else {
            return false;
        };
        state.library.is_playlist_owned_by(id, user)
    }

    pub(super) fn unfollow_playlist(&self, id: String) {
        let api = self.app_model.api();
        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(library::set_saved(&api, ItemId::Playlist(id), false)
                .await?
                .into())
        })
    }

    fn playlist_title(&self, id: &str) -> Option<String> {
        self.get_playlists()
            .into_iter()
            .find_map(|dest| match dest {
                NavigationPanelDestination::Playlist(summary) if summary.id == id => {
                    Some(summary.title)
                }
                _ => None,
            })
    }

    pub(super) fn play_playlist(&self, id: String) {
        self.start_playlist(id, false);
    }

    pub(super) fn shuffle_playlist(&self, id: String) {
        self.start_playlist(id, true);
    }

    fn start_playlist(&self, id: String, shuffle: bool) {
        let name = self.playlist_title(&id);
        self.start(SongsSource::Playlist(id), name, shuffle);
    }

    fn start(&self, source: SongsSource, name: Option<String>, shuffle: bool) {
        let context = PlayContext::whole(source, name);
        self.dispatcher
            .dispatch_many(playback_actions(context.play(None, Some(shuffle))));
    }

    fn title_pinned_with(&self, kind: PinKind, id: &str) -> Option<String> {
        let state = self.app_model.get_state();
        let pin = state
            .library
            .pins()
            .iter()
            .find(|pin| pin.kind == kind && pin.id == id)?;
        state.library.pin_title(pin)
    }

    pub(super) fn queue_playlist(&self, id: String) {
        let source = SongsSource::Playlist(id);
        queue_source_tracks(&self.app_model, &self.dispatcher, source);
    }

    /// Play a pinned album from its first track, or shuffled.
    pub(super) fn play_album(&self, id: String, shuffle: bool) {
        let name = self.title_pinned_with(PinKind::Album, &id);
        self.start(SongsSource::Album(id), name, shuffle);
    }

    /// Play a pinned artist's top tracks, or shuffle them.
    pub(super) fn play_artist(&self, id: String, shuffle: bool) {
        let name = self.title_pinned_with(PinKind::Artist, &id);
        self.start(SongsSource::Artist(id), name, shuffle);
    }

    /// Whether a pinned item is in the user's library. Removing an album,
    /// artist or track unpins it, so pinned ones are saved.
    pub(super) fn is_in_library(&self, kind: PinKind, id: &str) -> bool {
        kind != PinKind::Playlist || self.app_model.get_state().library.has_playlist(id)
    }

    pub(super) fn is_pinned(&self, kind: PinKind, id: &str) -> bool {
        self.app_model.get_state().library.is_pinned(kind, id)
    }

    pub(super) fn remove_from_library(&self, kind: PinKind, id: String) {
        let item = match kind {
            PinKind::Playlist => ItemId::Playlist(id),
            PinKind::Album => ItemId::Album(id),
            PinKind::Artist => ItemId::Artist(id),
            PinKind::Track => ItemId::Tracks(vec![id]),
        };
        let api = self.app_model.api();
        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(library::set_saved(&api, item, false).await?.into())
        });
    }

    pub(super) fn navigate(&self, dest: NavigationPanelDestination) {
        let actions = match dest {
            NavigationPanelDestination::Library
            | NavigationPanelDestination::SavedTracks
            | NavigationPanelDestination::NowPlaying
            | NavigationPanelDestination::SavedPlaylists
            | NavigationPanelDestination::SavedArtists => {
                let page = dest.home_page().expect("home destination");
                vec![NavigationAction::Go(Route::Home(page)).into()]
            }
            NavigationPanelDestination::Playlist(PlaylistSummary { id, .. }) => {
                vec![AppAction::ViewPlaylist(id)]
            }
            NavigationPanelDestination::Album { id, .. } => vec![AppAction::ViewAlbum(id)],
            NavigationPanelDestination::Artist { id, .. } => vec![AppAction::ViewArtist(id)],
            NavigationPanelDestination::Track { id, .. } => {
                self.view_track_album(id);
                return;
            }
        };
        self.dispatcher.dispatch_many(actions);
    }

    /// Open the album page of a track, as there is no track detail page.
    fn view_track_album(&self, id: String) {
        let api = self.app_model.api();
        dispatch_api_read(&self.dispatcher, move |tag| async move {
            let track = api.get_track(&id, tag).await?;
            Ok(AppAction::ViewAlbum(
                track.album.map(|a| a.rri.id).unwrap_or_default(),
            ))
        });
    }

    /// Play a single track in the context of its album.
    pub(super) fn play_track(&self, id: String) {
        let api = self.app_model.api();
        dispatch_api_read_many(&self.dispatcher, move |tag| async move {
            let track = api.get_track(&id, tag).await?;
            let context = match track.album {
                Some(album) => {
                    PlayContext::whole(SongsSource::Album(album.rri.id), Some(album.name))
                }
                None => PlayContext {
                    source: SongsSource::Search(track.title),
                    name: None,
                    tracks: vec![id.clone()],
                },
            };
            Ok(playback_actions(context.play(Some(id), Some(false))))
        });
    }

    pub(super) fn toggle_pin_playlist(&self, id: &str) {
        let library = &self.app_model.get_state().library;
        let action = if library.is_pinned(PinKind::Playlist, id) {
            LibraryAction::Unpin(PinKind::Playlist, id.to_string())
        } else {
            let title = library.playlist(id).map(|p| p.title.clone());
            LibraryAction::Pin(Pin::new(PinKind::Playlist, id, title))
        };
        self.dispatcher.dispatch(action.into());
    }

    pub(super) fn unpin(&self, kind: PinKind, id: &str) {
        self.dispatcher
            .dispatch(LibraryAction::Unpin(kind, id.to_string()).into());
    }

    /// Rebuild the pinned and playlist rows of `list_store`.
    ///
    /// The store holds, in order: the fixed library entries, the pinned rows,
    /// the fixed "Playlists" header, then the playlists. `num_fixed_entries`
    /// counts the fixed rows only. The fixed rows are kept rather than
    /// rebuilt.
    pub fn apply_navigation_panel_items(
        &self,
        list_store: &gio::ListStore,
        num_fixed_entries: u32,
    ) {
        let (pinned, playlists) = self.build_navigation_panel_items();
        let position = |id: &str| {
            (0..list_store.n_items()).find(|&i| {
                list_store
                    .item(i)
                    .and_downcast::<NavigationPanelItem>()
                    .is_some_and(|item| item.id() == id)
            })
        };
        let Some(header) = position(SAVED_PLAYLISTS_SECTION) else {
            return;
        };
        let pinned_start = position(PINNED_SECTION).unwrap_or(header);
        let num_header_entries = num_fixed_entries.saturating_sub(pinned_start);
        list_store.splice(pinned_start, header - pinned_start, pinned.as_slice());

        let playlists_start = pinned_start + pinned.len() as u32 + num_header_entries;
        list_store.splice(
            playlists_start,
            list_store.n_items().saturating_sub(playlists_start),
            playlists.as_slice(),
        );
    }

    fn pinned_title(&self, pin: &Pin) -> String {
        let title = self.app_model.get_state().library.pin_title(pin);
        title.unwrap_or_else(|| match pin.kind {
            PinKind::Playlist => gettext("Pinned Playlist"),
            PinKind::Album => gettext("Pinned Album"),
            PinKind::Artist => gettext("Pinned Artist"),
            PinKind::Track => gettext("Pinned Track"),
        })
    }

    fn to_pinned_item(&self, pin: &Pin) -> NavigationPanelItem {
        let title = self.pinned_title(pin);
        let id = pin.id.clone();
        NavigationPanelItem::from_destination(match pin.kind {
            PinKind::Playlist => {
                NavigationPanelDestination::Playlist(PlaylistSummary { id, title })
            }
            PinKind::Album => NavigationPanelDestination::Album { id, title },
            PinKind::Artist => NavigationPanelDestination::Artist { id, title },
            PinKind::Track => NavigationPanelDestination::Track { id, title },
        })
    }

    /// Build the pinned rows (with their section header) and the playlist rows.
    fn build_navigation_panel_items(&self) -> (Vec<NavigationPanelItem>, Vec<NavigationPanelItem>) {
        let state = self.app_model.get_state();
        let library = &state.library;
        if !is_enabled(FeatureFlag::PinnedObjects) {
            let playlists = library.playlists().iter();
            return (vec![], playlists.map(Self::to_item).collect());
        }
        let mut pinned = vec![];
        if !library.pins().is_empty() {
            pinned.push(NavigationPanelItem::pinned_section());
            pinned.extend(library.pins().iter().map(|pin| self.to_pinned_item(pin)));
        }
        let playlists = library.unpinned_playlists().map(Self::to_item).collect();
        (pinned, playlists)
    }

    fn to_item(playlist: &Playlist) -> NavigationPanelItem {
        NavigationPanelItem::from_destination(Self::map_to_destination(playlist))
    }
}
