mod library;
mod playback;
mod session;

pub use library::{
    LibraryChanged, LibraryFields, PlaylistFields, PlaylistUpdated, SearchChanged, SearchFields,
};
pub use playback::{
    DeviceFields, DeviceRequest, DevicesChanged, PlaybackChanged, PlaybackFields, PlaybackSnapshot,
    PlaybackStatus, PositionOrigin, QueueChanged,
};
pub use session::{EngineStatus, LoginMethod, SessionChanged, SessionFields, SessionStatus};

pub use riff_config::{Setting, SettingSet, SettingsChanged};

pub use crate::navigation::{NavigationChanged, NavigationFields};
pub use crate::selection::{SelectionChanged, SelectionFields};

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Playback(PlaybackChanged),
    Queue(QueueChanged),
    Devices(DevicesChanged),
    Playlist(PlaylistUpdated),
    Library(LibraryChanged),
    Search(SearchChanged),
    Session(SessionChanged),
    Navigation(NavigationChanged),
    DeviceRequest(DeviceRequest),
    Saved(SavedChanged),
    Selection(SelectionChanged),
    Settings(SettingsChanged),
    PlayerSettingsChanged,
    TokenRejected,
    TokenRefreshed,
    Error(ErrorKind),
    Notice(Notice),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    AddedToQueue,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemId {
    Album(String),
    Playlist(String),
    Artist(String),
    Tracks(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedChanged {
    pub item: ItemId,
    pub saved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    DrmBlocked,
    Device(crate::remote::Failure),
}

impl Event {
    pub fn merge(&mut self, other: Event) -> Result<(), Event> {
        match (self, other) {
            (Event::Playback(a), Event::Playback(b)) => {
                a.changed |= b.changed;
                a.state = b.state;
                a.position_origin = match (a.position_origin, b.position_origin) {
                    (Some(PositionOrigin::User), _) => Some(PositionOrigin::User),
                    (a, b) => b.or(a),
                };
            }
            (Event::Queue(_), Event::Queue(_)) => {}
            (Event::Devices(a), Event::Devices(b)) => a.changed |= b.changed,
            (Event::Playlist(a), Event::Playlist(b)) if a.id == b.id => a.changed |= b.changed,
            (Event::Library(a), Event::Library(b)) => a.changed |= b.changed,
            (Event::Search(a), Event::Search(b)) => a.changed |= b.changed,
            (Event::Selection(a), Event::Selection(b)) => {
                *a = SelectionChanged {
                    changed: a.changed | b.changed,
                    ..b
                };
            }
            (Event::Settings(a), Event::Settings(b)) => a.changed |= b.changed,
            (Event::Session(a), Event::Session(b)) => {
                *a = SessionChanged {
                    changed: a.changed | b.changed,
                    ..b
                };
            }
            (_, other) => return Err(other),
        }
        Ok(())
    }
}

pub fn coalesce(events: impl IntoIterator<Item = Event>) -> Vec<Event> {
    let mut merged: Vec<Event> = vec![];
    'next: for mut event in events {
        for existing in merged.iter_mut() {
            match existing.merge(event) {
                Ok(()) => continue 'next,
                Err(rejected) => event = rejected,
            }
        }
        merged.push(event);
    }
    merged
}

impl_from!(Event {
    PlaybackChanged => Playback,
    QueueChanged => Queue,
    DevicesChanged => Devices,
    PlaylistUpdated => Playlist,
    LibraryChanged => Library,
    SearchChanged => Search,
    SessionChanged => Session,
    NavigationChanged => Navigation,
    SavedChanged => Saved,
    SelectionChanged => Selection,
    SettingsChanged => Settings,
    ErrorKind => Error,
    Notice => Notice,
    DeviceRequest => DeviceRequest,
});

#[cfg(test)]
mod tests {
    use super::*;

    fn playback(changed: PlaybackFields, volume: f64) -> Event {
        PlaybackChanged {
            changed,
            state: PlaybackSnapshot {
                volume,
                ..Default::default()
            },
            position_origin: None,
        }
        .into()
    }

    #[test]
    fn one_event_per_object() {
        let events = coalesce([
            playback(PlaybackFields::SHUFFLE, 0.1),
            QueueChanged.into(),
            playback(PlaybackFields::VOLUME, 0.5),
            QueueChanged.into(),
        ]);
        assert_eq!(
            events,
            [
                playback(PlaybackFields::SHUFFLE | PlaybackFields::VOLUME, 0.5),
                QueueChanged.into(),
            ]
        );
    }

    #[test]
    fn a_seek_by_the_user_stays_one_when_merged() {
        let moved = |origin| -> Event {
            PlaybackChanged {
                changed: PlaybackFields::POSITION,
                state: PlaybackSnapshot::default(),
                position_origin: Some(origin),
            }
            .into()
        };
        let events = coalesce([moved(PositionOrigin::User), moved(PositionOrigin::Player)]);
        let [Event::Playback(merged)] = &events[..] else {
            panic!("one event: {events:?}");
        };
        assert_eq!(merged.position_origin, Some(PositionOrigin::User));
    }

    #[test]
    fn different_objects_stay_apart() {
        let playlist = |id: &str, changed| Event::from(PlaylistUpdated::new(id, changed));
        let events = coalesce([
            playlist("a", PlaylistFields::INFO),
            playlist("b", PlaylistFields::INFO),
            playlist("a", PlaylistFields::TRACKS),
        ]);
        assert_eq!(
            events,
            [
                playlist("a", PlaylistFields::INFO | PlaylistFields::TRACKS),
                playlist("b", PlaylistFields::INFO),
            ]
        );
    }

    #[test]
    fn setting_changes_merge_into_one_event() {
        let events = coalesce([
            SettingsChanged::from(Setting::Pan).into(),
            QueueChanged.into(),
            SettingsChanged::from(Setting::EqBand2).into(),
        ]);
        let both: SettingSet = [Setting::Pan, Setting::EqBand2].into_iter().collect();
        assert_eq!(
            events,
            [
                SettingsChanged { changed: both }.into(),
                Event::from(QueueChanged)
            ]
        );
    }

    #[test]
    fn merged_session_changes_carry_the_latest_engine() {
        let session = |changed, engine| -> Event {
            SessionChanged {
                changed,
                status: SessionStatus::LoggedIn,
                connection_lost: false,
                engine,
            }
            .into()
        };
        let events = coalesce([
            session(SessionFields::ENGINE, EngineStatus::Installing),
            session(SessionFields::STATUS, EngineStatus::Ready),
        ]);
        assert_eq!(
            events,
            [session(
                SessionFields::ENGINE | SessionFields::STATUS,
                EngineStatus::Ready
            )]
        );
    }

    #[test]
    fn notices_are_not_merged() {
        let events = coalesce([
            Event::PlayerSettingsChanged,
            Event::PlayerSettingsChanged,
            ErrorKind::DrmBlocked.into(),
        ]);
        assert_eq!(events.len(), 3);
    }
}
