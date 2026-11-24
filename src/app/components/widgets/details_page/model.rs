use std::cell::Ref;
use std::collections::HashSet;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use gio::prelude::*;
use gio::SimpleActionGroup;

use crate::app::components::{
    add_tracks_to_playlist, build_song_menu, dispatch_api_call, dispatch_api_call_many,
    queue_source_tracks, PlaylistPicker, QueueMenuEntry, SongActions, REMOVE_FROM_PLAYLIST_ACTION,
};
use crate::app::dispatch::Dispatcher;
use crate::app::models::{SongListModel, Track};
use crate::app::state::Action;
use crate::app::state::PinKind;
use crate::app::state::{
    playback_actions, PlayContext, PlaybackAction, SelectionAction, SelectionContext,
    SelectionState,
};
use crate::app::state::{ItemId, LibraryAction, LikedTracks, Pin};
use crate::app::{AppAction, AppModel, AppState, SongsSource};
use riff_config::feature_flag::{self, FeatureFlag};
use riff_core::library;

#[macro_export]
macro_rules! impl_track_list_model_base {
    () => {
        fn is_paused(&self) -> bool {
            self.base.is_paused()
        }
        fn current_song_id(&self) -> Option<String> {
            self.base.current_song_id()
        }
        fn select_song(&self, id: &str) {
            self.select_songs(&[id.to_string()]);
        }
        fn select_songs(&self, ids: &[String]) {
            self.select_songs_from_list(&TrackListModel::song_list_model(self), ids);
        }
        fn deselect_song(&self, id: &str) {
            self.base.deselect_song(id);
        }
        fn selection(&self) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
            self.base.selection()
        }
        fn is_song_liked(&self, id: &str) -> bool {
            self.base.is_song_liked(id)
        }
        fn update_liked(&self, event: &AppEvent) {
            if let Some(event) = event.core() {
                self.base.liked.apply(event);
            }
        }
        fn toggle_song_like(&self, id: &str) {
            let songs = TrackListModel::song_list_model(self);
            self.base.toggle_song_like(&songs, id);
        }
        fn pinned_song_ids(&self) -> Option<std::collections::HashSet<String>> {
            self.base.pinned_song_ids()
        }
        fn toggle_song_pin(&self, song: &Track) {
            self.base.toggle_song_pin(song);
        }
        fn skip_explicit(&self) -> bool {
            self.base.skip_explicit()
        }
    };
}

#[macro_export]
macro_rules! impl_source_page {
    () => {
        fn source_is_playing(&self) -> bool {
            self.base
                .is_playing_source(PageModel::songs_source(self).as_ref())
        }

        fn start_play(&self, id: &str) {
            let songs = TrackListModel::song_list_model(self);
            match songs.find_index(id) {
                Some(index) => TrackListModel::play_song_at(self, index, id),
                None => error!("Failed to play track {id}"),
            }
        }

        fn toggle_play(&self) {
            self.base.toggle_playback(self.source_is_playing(), || {
                TrackListModel::play_context(self)
            });
        }

        fn shuffle_play(&self) {
            self.base
                .start_playback(true, TrackListModel::play_context(self));
        }

        fn queue_all(&self) {
            if let Some(source) = PageModel::songs_source(self) {
                self.base
                    .queue_source(source, &TrackListModel::song_list_model(self));
            }
        }

        fn on_share_clicked(&self) {
            if let Some(url) = PageModel::songs_source(self).and_then(|s| s.spotify_url()) {
                self.base.share_link(&url);
            }
        }
    };
}

#[macro_export]
macro_rules! impl_track_list_header_bar {
    ($model:ty) => {
        impl $crate::app::components::SimpleHeaderBarModel for $model {
            fn select_all(&self) {
                let songs = $crate::app::components::TrackListModel::song_list_model(self);
                self.dispatcher
                    .dispatch($crate::app::state::SelectionAction::Select(songs.collect()).into());
            }

            fn selectable_keys(&self) -> Vec<String> {
                $crate::app::components::TrackListModel::song_list_model(self).ids()
            }
        }
    };
}

#[macro_export]
macro_rules! impl_source_track_list {
    () => {
        fn load_more(&self) {
            PageModel::load_more(self);
        }

        fn play_song_at(&self, _pos: usize, id: &str) {
            self.play_in_context(TrackListModel::play_context(self), id);
        }

        fn play_context(&self) -> Option<$crate::app::state::PlayContext> {
            Some($crate::app::components::source_play_context(
                PageModel::songs_source(self)?,
                PageModel::context_name(self),
                &TrackListModel::song_list_model(self),
            ))
        }

        fn actions_for(
            &self,
            _row: &$crate::app::models::SongModel,
            song: &$crate::app::models::Track,
        ) -> Option<gio::SimpleActionGroup> {
            Some(
                self.base
                    .track_actions(song, PageModel::songs_source(self).as_ref()),
            )
        }

        fn playlist_picker(&self, song: &$crate::app::models::Track) -> Option<gtk::Widget> {
            self.base.playlist_picker(song)
        }

        fn menu_opening(&self, actions: &gio::SimpleActionGroup) {
            self.base
                .refresh_track_actions(actions, PageModel::songs_source(self).as_ref());
        }

        fn menu_for(
            &self,
            _row: &$crate::app::models::SongModel,
            song: &$crate::app::models::Track,
            liked: bool,
            pinned: Option<bool>,
        ) -> Option<gio::MenuModel> {
            Some($crate::app::components::track_menu(
                song,
                PageModel::songs_source(self).as_ref(),
                liked,
                pinned,
            ))
        }
    };
}

pub fn source_play_context(
    source: SongsSource,
    name: Option<String>,
    songs: &SongListModel,
) -> PlayContext {
    let tracks = songs
        .collect()
        .into_iter()
        .filter(|t: &Track| t.playable)
        .map(|t| t.rri.id)
        .collect();
    PlayContext {
        source,
        name,
        tracks,
    }
}

/// No "View Album" on its album's page, nor "More from" on its artist's.
pub fn track_menu(
    song: &Track,
    source: Option<&SongsSource>,
    liked: bool,
    pinned: Option<bool>,
) -> gio::MenuModel {
    let show_view_album = !matches!(source, Some(SongsSource::Album(_)));
    let artist = match source {
        Some(SongsSource::Artist(id)) => Some(id.as_str()),
        _ => None,
    };
    build_song_menu(
        song,
        show_view_album,
        artist,
        QueueMenuEntry::Add,
        Some(liked),
        pinned,
        matches!(source, Some(SongsSource::Playlist(_))),
    )
}

pub struct DetailsPageModel {
    pub id: String,
    pub app_model: Rc<AppModel>,
    pub dispatcher: Dispatcher,
    pub liked: LikedTracks,
}

impl DetailsPageModel {
    pub fn new(id: String, app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            id,
            app_model,
            dispatcher,
            liked: LikedTracks::default(),
        }
    }

    pub fn new_without_id(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self::new(String::new(), app_model, dispatcher)
    }

    pub fn state(&self) -> Ref<'_, AppState> {
        self.app_model.get_state()
    }

    #[allow(dead_code)]
    pub fn dispatcher(&self) -> &Dispatcher {
        &self.dispatcher
    }

    /// Copy a shareable link to the clipboard and show a confirmation toast.
    pub fn is_playing_source(&self, source: Option<&SongsSource>) -> bool {
        source.is_some() && self.state().playback.current_source().as_ref() == source
    }

    /// Every page of a paginated source, else `songs` as listed.
    pub fn queue_source(&self, source: SongsSource, songs: &SongListModel) {
        if source.is_paginated() {
            queue_source_tracks(&self.app_model, &self.dispatcher, source);
        } else {
            let tracks: Vec<Track> = songs.collect();
            let tracks = tracks.into_iter().filter(|t| t.playable).collect();
            self.dispatcher
                .dispatch(AppAction::Core(Action::QueueTracks(tracks)));
        }
    }

    pub fn track_actions(&self, song: &Track, source: Option<&SongsSource>) -> SimpleActionGroup {
        let group = SimpleActionGroup::new();
        for action in song.make_artist_actions(self.dispatcher.clone()) {
            group.add_action(&action);
        }
        if !matches!(source, Some(SongsSource::Album(_))) {
            group.add_action(&song.make_album_action(self.dispatcher.clone()));
        }
        group.add_action(&song.make_link_action());
        for action in song.make_queue_actions(self.dispatcher.clone()) {
            group.add_action(&action);
        }
        if let Some(action) = self.remove_from_playlist_action(song, source) {
            group.add_action(&action);
        }
        group
    }

    pub fn owns_playlist(&self, id: &str) -> bool {
        let state = self.state();
        state
            .session
            .user
            .as_deref()
            .is_some_and(|user| state.library.is_playlist_owned_by(id, user))
    }

    pub fn editable_playlist(&self, source: Option<&SongsSource>) -> Option<String> {
        match source {
            Some(SongsSource::Playlist(id)) if self.owns_playlist(id) => Some(id.clone()),
            _ => None,
        }
    }

    pub fn refresh_track_actions(&self, actions: &SimpleActionGroup, source: Option<&SongsSource>) {
        let remove = actions
            .lookup_action(REMOVE_FROM_PLAYLIST_ACTION)
            .and_downcast::<gio::SimpleAction>();
        if let Some(remove) = remove {
            remove.set_enabled(self.editable_playlist(source).is_some());
        }
    }

    fn remove_from_playlist_action(
        &self,
        song: &Track,
        source: Option<&SongsSource>,
    ) -> Option<gio::SimpleAction> {
        let Some(SongsSource::Playlist(id)) = source else {
            return None;
        };
        let id = id.clone();
        let uri = song.rri.uri.clone()?;
        let api = self.app_model.api();
        let dispatcher = self.dispatcher.clone();
        let action = gio::SimpleAction::new(REMOVE_FROM_PLAYLIST_ACTION, None);
        action.set_enabled(false);
        action.connect_activate(move |_, _| {
            let api = Arc::clone(&api);
            let id = id.clone();
            let uris = vec![uri.clone()];
            dispatch_api_call(&dispatcher, move || async move {
                Ok(library::remove_from_playlist(&api, id, uris.clone())
                    .await?
                    .into())
            })
        });
        Some(action)
    }

    pub fn playlist_picker(&self, song: &Track) -> Option<gtk::Widget> {
        song.rri.uri.as_ref()?;
        let song = song.clone();
        let playlists = self.state().owned_playlists();
        let app_model = Rc::clone(&self.app_model);
        let dispatcher = self.dispatcher.clone();
        let picker = PlaylistPicker::new(playlists, move |playlist| {
            add_tracks_to_playlist(
                &app_model,
                &dispatcher,
                playlist.id.clone(),
                vec![song.clone()],
                None,
            );
        });
        Some(picker.upcast())
    }

    pub fn share_link(&self, link: &str) {
        crate::app::components::copy_link_to_clipboard(link);
        self.dispatcher
            .dispatch(AppAction::ShowNotification(gettextrs::gettext(
                "Link copied to clipboard",
            )));
    }

    pub fn is_paused(&self) -> bool {
        !self.state().playback.is_playing()
    }

    pub fn is_playing(&self) -> bool {
        self.state().playback.is_playing()
    }

    pub fn current_song_id(&self) -> Option<String> {
        self.state().playback.current_song_id()
    }

    pub fn skip_explicit(&self) -> bool {
        self.state().playback.skip_explicit()
    }

    pub fn deselect_song(&self, id: &str) {
        self.dispatcher
            .dispatch(SelectionAction::Deselect(vec![id.to_string()]).into());
    }

    pub fn selection(&self) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
        self.selection_in(false)
    }

    pub fn selection_in(
        &self,
        queue: bool,
    ) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
        let selection = self.app_model.map_state(|s| &s.selection);
        if selection.is_selecting_in(!queue) {
            return None;
        }
        Some(Box::new(selection))
    }

    pub fn select_songs_from_list(&self, song_list: &SongListModel, ids: &[String]) {
        let songs: Vec<Track> = ids
            .iter()
            .filter_map(|id| song_list.get(id))
            .map(|song| song.description().clone())
            .collect();
        if !songs.is_empty() {
            self.dispatcher
                .dispatch(SelectionAction::Select(songs).into());
        }
    }

    pub fn enable_selection_with_context(&self, context: SelectionContext) -> bool {
        self.dispatcher
            .dispatch(AppAction::Core(Action::EnableSelection(context)));
        true
    }

    /// Toggle play/pause, or start the source over with `context`.
    pub fn toggle_playback(
        &self,
        source_is_playing: bool,
        context: impl FnOnce() -> Option<PlayContext>,
    ) {
        if !source_is_playing {
            self.start_playback(false, context());
        } else if self.is_playing() {
            self.dispatcher.dispatch(PlaybackAction::Pause.into());
        } else {
            self.dispatcher.dispatch(PlaybackAction::Play.into());
        }
    }

    /// Plays `context` from the start, shuffled or not.
    pub fn start_playback(&self, shuffle: bool, context: Option<PlayContext>) {
        if let Some(context) = context {
            self.dispatcher
                .dispatch_many(playback_actions(context.play(None, Some(shuffle))));
        }
    }

    pub fn play_in_context(&self, context: Option<PlayContext>, id: &str) {
        if let Some(context) = context {
            self.dispatcher
                .dispatch_many(playback_actions(context.play(Some(id.to_string()), None)));
        }
    }

    pub fn is_song_liked(&self, id: &str) -> bool {
        self.liked.is_liked(id)
    }

    /// IDs of the tracks pinned to the navigation panel, or `None` when pinning is
    /// disabled.
    pub fn pinned_song_ids(&self) -> Option<HashSet<String>> {
        if !feature_flag::is_enabled(FeatureFlag::PinnedObjects) {
            return None;
        }
        let state = self.app_model.get_state();
        Some(
            state
                .library
                .pins()
                .ids(PinKind::Track)
                .map(String::from)
                .collect(),
        )
    }

    pub fn toggle_song_pin(&self, song: &Track) {
        let id = song.rri.id.clone();
        if self
            .app_model
            .get_state()
            .library
            .is_pinned(PinKind::Track, &id)
        {
            self.dispatcher
                .dispatch(LibraryAction::Unpin(PinKind::Track, id).into());
            return;
        }
        let pin = LibraryAction::Pin(Pin::new(
            PinKind::Track,
            id.clone(),
            Some(song.title.clone()),
        ));
        if self.is_song_liked(&id) {
            self.dispatcher.dispatch(pin.into());
            return;
        }
        let api = self.app_model.api();
        dispatch_api_call_many(&self.dispatcher, move || async move {
            let liked = library::set_saved(&api, ItemId::Tracks(vec![id]), true).await?;
            Ok(vec![liked.into(), pin.into()])
        });
    }

    pub fn toggle_song_like(&self, song_list: &SongListModel, id: &str) {
        let Some(song) = song_list.get(id) else {
            return;
        };
        let song_id = song.get_id();
        let api = self.app_model.api();
        let liked = !self.is_song_liked(id);
        dispatch_api_call(&self.dispatcher, move || async move {
            Ok(
                library::set_saved(&api, ItemId::Tracks(vec![song_id]), liked)
                    .await?
                    .into(),
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Arc;

    use crate::app::components::details_page::is_playback_event;
    use crate::app::models::*;
    use crate::app::state::{
        LibraryChanged, LibraryFields, PlaybackChanged, PlaybackFields, PlaybackSnapshot,
        PlaybackStatus,
    };
    use riff_core::playback::PlayRequest;

    fn playback_event(changed: PlaybackFields, status: PlaybackStatus) -> AppEvent {
        PlaybackChanged {
            changed,
            state: PlaybackSnapshot {
                status,
                ..Default::default()
            },
            position_origin: None,
        }
        .into()
    }
    use crate::app::AppEvent;

    struct TestDispatcher {
        dispatcher: Dispatcher,
        receiver: RefCell<futures::channel::mpsc::UnboundedReceiver<AppAction>>,
        received: RefCell<Vec<AppAction>>,
    }

    impl TestDispatcher {
        fn new() -> Self {
            let (sender, receiver) = futures::channel::mpsc::unbounded();
            Self {
                dispatcher: Dispatcher::new(sender),
                receiver: RefCell::new(receiver),
                received: RefCell::new(Vec::new()),
            }
        }

        fn dispatcher(&self) -> Dispatcher {
            self.dispatcher.clone()
        }

        fn pump(&self) {
            while let Ok(Some(action)) = self.receiver.borrow_mut().try_next() {
                self.received.borrow_mut().push(action);
            }
        }

        fn dispatched(&self) -> Vec<AppAction> {
            self.pump();
            self.received.borrow().clone()
        }

        fn last_action(&self) -> Option<AppAction> {
            self.pump();
            self.received.borrow().last().cloned()
        }

        fn clear(&self) {
            self.pump();
            self.received.borrow_mut().clear();
        }
    }

    /// An `ApiService` that is never called; `AppModel` just needs one to exist.
    /// No request is issued and no disk is touched, since cache directories are
    /// created on first write. Shared, to avoid an HTTP client per test.
    fn test_api_service() -> Arc<riff_api::ApiService> {
        thread_local! {
            static API: Arc<riff_api::ApiService> = {
                let token_provider: Arc<dyn riff_api::TokenProvider> =
                    Arc::new(|| None::<String>);
                Arc::new(riff_api::spotify_service(
                    Arc::clone(&token_provider),
                    token_provider,
                    1024 * 1024,
                    1024 * 1024,
                ))
            };
        }
        API.with(Arc::clone)
    }

    fn make_model() -> (DetailsPageModel, TestDispatcher) {
        let dispatcher = TestDispatcher::new();
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        let model =
            DetailsPageModel::new("test-id".to_string(), app_model, dispatcher.dispatcher());
        (model, dispatcher)
    }

    fn make_model_playing() -> (DetailsPageModel, TestDispatcher) {
        let dispatcher = TestDispatcher::new();
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        app_model.update_state(
            riff_core::playback::test_playing_from(None, song("s1"), &[], &[song("s2")]).into(),
        );
        let model =
            DetailsPageModel::new("test-id".to_string(), app_model, dispatcher.dispatcher());
        (model, dispatcher)
    }

    fn song(id: &str) -> Track {
        make_track(id)
    }

    fn make_song_list(songs: Vec<Track>) -> SongListModel {
        let mut list = SongListModel::new(50);
        let _ = list.add(Page {
            items: songs,
            offset: Some(0),
            total: None,
            next_cursor: None,
        });
        list
    }

    #[test]
    fn test_is_playback_event_paused() {
        let event = playback_event(PlaybackFields::STATUS, PlaybackStatus::Paused);
        assert_eq!(is_playback_event(&event), Some(false));
    }

    #[test]
    fn test_is_playback_event_resumed() {
        let event = playback_event(PlaybackFields::STATUS, PlaybackStatus::Playing);
        assert_eq!(is_playback_event(&event), Some(true));
    }

    #[test]
    fn test_is_playback_event_track_changed() {
        let event = playback_event(PlaybackFields::TRACK, PlaybackStatus::Playing);
        assert_eq!(is_playback_event(&event), Some(true));
    }

    #[test]
    fn test_is_playback_event_non_playback() {
        let event = AppEvent::from(LibraryChanged {
            changed: LibraryFields::PLAYLISTS,
        });
        assert_eq!(is_playback_event(&event), None);
    }

    #[test]
    fn test_initial_state_is_paused() {
        let (model, _) = make_model();
        assert!(model.is_paused());
        assert!(!model.is_playing());
        assert_eq!(model.current_song_id(), None);
    }

    #[test]
    fn test_playing_state() {
        let (model, _) = make_model_playing();
        assert!(model.is_playing());
        assert!(!model.is_paused());
        assert_eq!(model.current_song_id(), Some("s1".to_string()));
    }

    #[test]
    fn test_deselect_song_dispatches() {
        let (model, dispatcher) = make_model();
        model.deselect_song("song-1");
        let action = dispatcher.last_action().unwrap();
        assert!(
            matches!(action, AppAction::Core(Action::Selection(SelectionAction::Deselect(ids))) if ids == vec!["song-1".to_string()])
        );
    }

    #[test]
    fn test_selection_returns_state() {
        let (model, _) = make_model();
        let sel = model.selection();
        assert!(sel.is_some());
        assert!(!sel.unwrap().is_selection_enabled());
    }

    #[test]
    fn test_select_songs_from_list_dispatches() {
        let (model, dispatcher) = make_model();
        let list = make_song_list(vec![song("a"), song("b")]);
        model.select_songs_from_list(&list, &["b".to_string()]);
        let action = dispatcher.last_action().unwrap();
        assert!(
            matches!(action, AppAction::Core(Action::Selection(SelectionAction::Select(songs))) if songs.len() == 1 && songs[0].rri.id == "b")
        );
    }

    #[test]
    fn test_select_songs_from_list_nonexistent() {
        let (model, dispatcher) = make_model();
        let list = make_song_list(vec![song("a")]);
        model.select_songs_from_list(&list, &["nonexistent".to_string()]);
        assert!(dispatcher.last_action().is_none());
    }

    #[test]
    fn test_enable_selection_with_context() {
        let (model, dispatcher) = make_model();
        let result = model.enable_selection_with_context(SelectionContext::Default);
        if result {
            let action = dispatcher.last_action().unwrap();
            assert!(matches!(
                action,
                AppAction::Core(Action::EnableSelection(SelectionContext::Default))
            ));
        }
    }

    #[test]
    fn test_source_helpers() {
        use crate::app::models::ArtistRef;
        let album = SongsSource::Album("al".into());
        let artist = SongsSource::Artist("ar".into());
        assert_eq!(
            album.spotify_url().as_deref(),
            Some("https://open.spotify.com/album/al")
        );
        assert_eq!(SongsSource::SavedTracks.spotify_url(), None);

        let mut track = song("t");
        track.artists = vec![ArtistRef {
            rri: ResourceId {
                id: "ar".into(),
                ..Default::default()
            },
            name: "Artist".into(),
        }];
        let entries = |source: Option<&SongsSource>| -> Vec<String> {
            let menu = track_menu(&track, source, false, None);
            (0..menu.n_items())
                .filter_map(|i| menu.item_link(i, gio::MENU_LINK_SECTION))
                .flat_map(|section| {
                    (0..section.n_items())
                        .filter_map(|j| {
                            section
                                .item_attribute_value(j, gio::MENU_ATTRIBUTE_ACTION, None)
                                .and_then(|v| v.get::<String>())
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let has = |source, action: &str| entries(source).iter().any(|a| a == action);
        assert!(has(None, "song.view_album") && has(None, "song.view_artist_ar"));
        assert!(!has(Some(&album), "song.view_album"));
        assert!(!has(Some(&artist), "song.view_artist_ar"));
        assert!(has(Some(&album), "song.queue"));

        let (model, dispatcher) = make_model();
        assert!(model
            .track_actions(&track, Some(&album))
            .lookup_action("view_album")
            .is_none());
        assert!(model
            .track_actions(&track, None)
            .lookup_action("view_album")
            .is_some());

        let mut unplayable = song("u");
        unplayable.playable = false;
        let list = make_song_list(vec![song("a"), unplayable]);
        let context = source_play_context(album.clone(), Some("A".into()), &list);
        assert_eq!(context.tracks, ["a"]);
        assert_eq!(context.name.as_deref(), Some("A"));

        model.queue_source(artist, &list);
        assert!(matches!(
            dispatcher.last_action(),
            Some(AppAction::Core(Action::QueueTracks(ref tracks))) if tracks.len() == 1
        ));
        assert!(!model.is_playing_source(Some(&album)));
        assert!(!model.is_playing_source(None));
    }

    fn play_request(actions: &[AppAction]) -> Option<PlayRequest> {
        actions.iter().find_map(|a| match a {
            AppAction::Core(Action::Playback(PlaybackAction::PlayFrom(request))) => {
                Some(request.clone())
            }
            _ => None,
        })
    }

    #[test]
    fn test_playback_helpers() {
        let artist = crate::app::SongsSource::Artist("a".into());
        let context = || {
            Some(PlayContext {
                source: artist.clone(),
                name: Some("An Artist".into()),
                tracks: vec!["x".into()],
            })
        };

        let (model, dispatcher) = make_model();
        model.toggle_playback(false, context);
        let actions = dispatcher.dispatched();
        assert!(matches!(
            actions[0],
            AppAction::Core(Action::Playback(PlaybackAction::SetContextName(..)))
        ));
        let request = play_request(&actions).unwrap();
        assert_eq!(request.source, Some(artist.clone()));
        assert_eq!((request.start, request.shuffle), (None, Some(false)));

        dispatcher.clear();
        model.start_playback(true, context());
        assert_eq!(
            play_request(&dispatcher.dispatched()).unwrap().shuffle,
            Some(true)
        );

        dispatcher.clear();
        model.toggle_playback(false, || None);
        model.play_in_context(None, "x");
        assert!(dispatcher.dispatched().is_empty());
        model.play_in_context(context(), "x");
        let request = play_request(&dispatcher.dispatched()).unwrap();
        assert_eq!(
            (request.start.as_deref(), request.shuffle),
            (Some("x"), None)
        );

        dispatcher.clear();
        model.toggle_playback(true, || panic!("should not start playback"));
        assert!(matches!(
            dispatcher.last_action(),
            Some(AppAction::Core(Action::Playback(PlaybackAction::Play)))
        ));
        let (model, dispatcher) = make_model_playing();
        model.toggle_playback(true, || panic!("should not start playback"));
        assert!(matches!(
            dispatcher.last_action(),
            Some(AppAction::Core(Action::Playback(PlaybackAction::Pause)))
        ));
    }

    #[test]
    fn test_new_stores_id() {
        let (model, _) = make_model();
        assert_eq!(model.id, "test-id");
    }

    #[test]
    fn test_new_without_id() {
        let dispatcher = TestDispatcher::new();
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        let model = DetailsPageModel::new_without_id(app_model, dispatcher.dispatcher());
        assert_eq!(model.id, "");
    }
}
