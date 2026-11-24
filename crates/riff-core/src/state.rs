use riff_api::models::Track;
use riff_config::{Setting, SettingsChanged};

use crate::browse::{Browser, BrowserAction, LibraryAction, NavigationAction, SearchAction};
use crate::events::{
    coalesce, DeviceRequest, Event, ItemId, LibraryChanged, LibraryFields, NavigationChanged,
    Notice, SelectionChanged,
};
use crate::library::{Library, PlaylistSummary};
use crate::navigation::Route;
use crate::play_queue::EntryKey;
use crate::playback::{PlaybackAction, PlaybackState};
use crate::remote::Intent;
use crate::selection::{SelectionAction, SelectionContext, SelectionState};
use crate::session::{Session, SessionAction};

#[derive(Clone, Debug)]
pub enum Action {
    Playback(PlaybackAction),
    Browser(BrowserAction),
    Selection(SelectionAction),
    Session(SessionAction),
    QueueSelection,
    QueueTracks(Vec<Track>),
    DequeueSelection,
    SaveSelection,
    UnsaveSelection,
    EnableSelection(SelectionContext),
    CancelSelection,
    ReloadPlayerSettings,
    SetNavigationHidden(bool),
    SettingsChanged(SettingsChanged),
    RecordTitle(Route, String),
}

#[derive(Default)]
pub struct AppState {
    pub playback: PlaybackState,
    pub browser: Browser,
    pub navigation_hidden: bool,
    pub selection: SelectionState,
    pub session: Session,
    pub library: Library,
}

impl AppState {
    pub fn apply(&mut self, action: Action) -> Vec<Event> {
        coalesce(self.reduce(action))
    }

    fn reduce(&mut self, action: Action) -> Vec<Event> {
        match action {
            Action::QueueSelection => {
                let tracks = self.selection.take_selection();
                let mut events = self.queue_tracks(tracks);
                events.push(SelectionChanged::mode(false).into());
                events
            }
            Action::QueueTracks(tracks) => self.queue_tracks(tracks),
            Action::DequeueSelection => {
                let entries: Vec<(EntryKey, String)> = self
                    .selection
                    .take_keyed_selection()
                    .into_iter()
                    .filter_map(|(key, track)| Some((key.parse().ok()?, track.rri.id)))
                    .collect();
                let mut events = self.playback.dequeue(&entries);
                events.push(SelectionChanged::mode(false).into());
                events
            }
            Action::SaveSelection => self.set_selection_saved(true),
            Action::UnsaveSelection => self.set_selection_saved(false),
            Action::EnableSelection(context) => {
                let had_selection = self.selection.count() > 0;
                let Some(active) = self.selection.set_mode(Some(context)) else {
                    return vec![];
                };
                let mut events = vec![SelectionChanged::mode(active).into()];
                if had_selection {
                    events.push(SelectionChanged::items(active).into());
                }
                events
            }
            Action::CancelSelection => match self.selection.set_mode(None) {
                Some(active) => vec![SelectionChanged::mode(active).into()],
                None => vec![],
            },
            Action::ReloadPlayerSettings => vec![Event::PlayerSettingsChanged],
            Action::Session(SessionAction::Logout) => {
                let mut events = self.library.clear();
                events.extend(self.session.apply(SessionAction::Logout));
                events
            }
            Action::Session(action) => self.session.apply(action),
            Action::Playback(PlaybackAction::PlayFrom(request)) => {
                let intent = request.intent(self.session.user.as_deref());
                vec![DeviceRequest::Control(intent).into()]
            }
            Action::Playback(action) => {
                let revision = self.playback.queue_revision();
                let mut events = self.playback.update(action);
                if self.playback.queue_revision() != revision {
                    events.extend(self.keep_selected_queue_rows());
                }
                events
            }
            Action::Browser(BrowserAction::Navigation(NavigationAction::Back))
                if !self.browser.navigator().can_go_back() && self.navigation_hidden =>
            {
                self.set_navigation_hidden(false)
            }
            Action::Browser(BrowserAction::Library(action)) => {
                let mut events = vec![];
                if let LibraryAction::RenamePlaylist(PlaylistSummary { id, title }) = &action {
                    let route = Route::Playlist(id.clone());
                    events.extend(
                        self.browser
                            .record_title(&route, title, self.navigation_hidden),
                    );
                }
                events.extend(self.library.apply(&action));
                events
            }
            Action::Browser(action) => self.browser.apply(&action, self.navigation_hidden),
            Action::RecordTitle(route, title) => {
                self.browser
                    .record_title(&route, &title, self.navigation_hidden)
            }
            Action::SetNavigationHidden(hidden) => self.set_navigation_hidden(hidden),
            Action::SettingsChanged(changed) => {
                let mut events = vec![Event::Settings(changed)];
                if changed.contains(Setting::FeaturePinnedObjects) {
                    events.push(
                        LibraryChanged {
                            changed: LibraryFields::PINS,
                        }
                        .into(),
                    );
                }
                events
            }
            Action::Selection(action) => self
                .selection
                .apply(action)
                .into_iter()
                .map(Event::from)
                .collect(),
        }
    }

    pub fn owned_playlists(&self) -> Vec<PlaylistSummary> {
        let Some(user) = self.session.user.as_deref() else {
            return vec![];
        };
        self.library
            .owned_playlists(user)
            .map(|p| p.clone().into())
            .collect()
    }

    fn set_navigation_hidden(&mut self, hidden: bool) -> Vec<Event> {
        self.navigation_hidden = hidden;
        vec![NavigationChanged::hidden(hidden).into()]
    }

    fn set_selection_saved(&mut self, saved: bool) -> Vec<Event> {
        let tracks = self
            .selection
            .take_selection()
            .into_iter()
            .map(|s| s.rri.id)
            .collect();
        let mut events = self.reduce(LibraryAction::SetSaved(ItemId::Tracks(tracks), saved).into());
        events.push(SelectionChanged::mode(false).into());
        events
    }

    fn keep_selected_queue_rows(&mut self) -> Vec<Event> {
        if !self.selection.is_selecting_in(true) {
            return vec![];
        }
        let playback = &self.playback;
        self.selection
            .retain(|key, track| {
                key.parse::<EntryKey>()
                    .is_ok_and(|key| playback.still_holds(key, &track.rri.id))
            })
            .into_iter()
            .map(Event::from)
            .collect()
    }

    fn queue_tracks(&mut self, tracks: Vec<Track>) -> Vec<Event> {
        if tracks.is_empty() {
            return vec![];
        }
        let mut events: Vec<Event> = tracks
            .into_iter()
            .map(|t| DeviceRequest::Control(Intent::AddToQueue(t.rri.id)).into())
            .collect();
        events.push(Notice::AddedToQueue.into());
        events
    }
}

impl_from!(Action {
    PlaybackAction => Playback,
    BrowserAction => Browser,
    SelectionAction => Selection,
    SessionAction => Session,
    NavigationAction => Browser,
    LibraryAction => Browser,
    SearchAction => Browser,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play_queue::SongsSource;
    use crate::playback::{test_playing_from, PlayContext};
    use crate::test_support::{playlist, requests, track};
    use riff_api::models::Playlist;

    fn changes(events: &[Event], fields: LibraryFields) -> bool {
        events
            .iter()
            .any(|e| matches!(e, Event::Library(l) if l.changed.intersects(fields)))
    }

    fn titled(id: &str, title: &str) -> Playlist {
        Playlist {
            title: title.into(),
            ..playlist(id)
        }
    }

    #[test]
    fn settings_changes_are_passed_on_and_pinning_shows_the_pins_again() {
        let mut state = AppState::default();
        let pan = SettingsChanged::from(Setting::Pan);
        let events = state.apply(Action::SettingsChanged(pan));
        assert_eq!(events, [Event::Settings(pan)]);

        let events = state.apply(Action::SettingsChanged(
            Setting::FeaturePinnedObjects.into(),
        ));
        assert!(changes(&events, LibraryFields::PINS));
    }

    #[test]
    fn saved_playlists_have_one_copy() {
        let mut state = AppState::default();
        let events = state.apply(LibraryAction::PlaylistsLoaded(vec![titled("a", "A")]).into());
        assert!(changes(&events, LibraryFields::PLAYLISTS));

        let save = || LibraryAction::SavePlaylist(Box::new(titled("b", "B"))).into();
        assert!(changes(&state.apply(save()), LibraryFields::PLAYLISTS));
        assert!(!changes(&state.apply(save()), LibraryFields::PLAYLISTS));

        state.apply(
            LibraryAction::RenamePlaylist(PlaylistSummary {
                id: "a".into(),
                title: "New".into(),
            })
            .into(),
        );
        assert_eq!(state.library.playlist("a").unwrap().title, "New");

        let events =
            state.apply(LibraryAction::SetSaved(ItemId::Playlist("b".into()), false).into());
        assert!(changes(&events, LibraryFields::PLAYLISTS));
        assert!(!state.library.has_playlist("b"));

        state.apply(SessionAction::Logout.into());
        assert!(state.library.playlists().is_empty());
    }

    #[test]
    fn selection_is_saved_by_id() {
        let mut state = AppState::default();
        state.apply(Action::EnableSelection(SelectionContext::Default));
        state.apply(SelectionAction::Select(vec![track("a")]).into());
        let events = state.apply(Action::SaveSelection);
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Saved(saved) if saved.item == ItemId::Tracks(vec!["a".into()]) && saved.saved
        )));
        assert!(!state.selection.is_selection_enabled());
    }

    #[test]
    fn queue_and_dequeue_selection() {
        let mut state = AppState::default();
        state.apply(Action::EnableSelection(SelectionContext::Default));
        state.apply(SelectionAction::Select(vec![track("a"), track("b")]).into());

        let events = state.apply(Action::QueueSelection);
        assert!(events.contains(&Notice::AddedToQueue.into()));
        assert_eq!(
            requests(&events),
            [
                Intent::AddToQueue("a".to_string()),
                Intent::AddToQueue("b".to_string())
            ]
        );
        assert!(!state.selection.is_selection_enabled());

        let playing = test_playing_from(None, track("1"), &[track("a"), track("b")], &[]);
        state.apply(playing.into());
        state.apply(Action::EnableSelection(SelectionContext::Queue));
        let key = state.playback.upcoming_keys()[1];
        let queued = state.playback.view_track(key).unwrap();
        state.apply(SelectionAction::SelectKeyed(vec![(key.to_string(), queued)]).into());
        let events = state.apply(Action::DequeueSelection);
        assert_eq!(
            requests(&events),
            [Intent::RemoveNext {
                indices: vec![1],
                revision: state.playback.queue_revision()
            }]
        );
    }

    #[test]
    fn selected_queue_rows_that_move_on_are_deselected() {
        let mut state = AppState::default();
        let with_revision = |action: PlaybackAction, revision: &str| match action {
            PlaybackAction::Show(mut view) => {
                view.playback.as_mut().unwrap().queue_revision = revision.to_string();
                PlaybackAction::Show(view)
            }
            action => action,
        };
        let queue = |current, next: &[Track], revision| {
            with_revision(test_playing_from(None, track(current), next, &[]), revision)
        };
        state.apply(queue("1", &[track("a"), track("b"), track("c")], "r1").into());
        state.apply(Action::EnableSelection(SelectionContext::Queue));
        let rows: Vec<_> = state
            .playback
            .upcoming_keys()
            .into_iter()
            .map(|key| (key.to_string(), state.playback.view_track(key).unwrap()))
            .collect();
        state.apply(SelectionAction::SelectKeyed(rows[1..].to_vec()).into());

        state.apply(queue("a", &[track("b"), track("c")], "r2").into());
        assert_eq!(state.selection.count(), 0, "b and c moved up a row");
        assert!(requests(&state.apply(Action::DequeueSelection)).is_empty());
    }

    #[test]
    fn playing_from_a_source_asks_the_device() {
        let mut state = AppState::default();
        state.apply(SessionAction::LoginSucceeded("me".into()).into());
        let context = PlayContext::whole(SongsSource::SavedTracks, Some("Liked".into()));
        let mut events = vec![];
        for action in context.play(None, Some(true)) {
            events.extend(state.apply(action.into()));
        }
        assert!(matches!(
            &requests(&events)[..],
            [Intent::Play {
                context: Some(riff_api::remote::RemoteContext::SavedTracks { user_id }),
                shuffle: Some(true),
                ..
            }] if user_id == "me"
        ));
    }
}
