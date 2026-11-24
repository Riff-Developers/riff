use gettextrs::gettext;

use riff_api::models::Link;

use crate::app::components::{labels, CardLayout, CardSize, SortOrder};
use crate::app::state::{
    browser_state::{PageFields, PageLoaded, ViewPreferencesChanged},
    BrowserAction, DevicesChanged, ErrorKind, Event, ItemId, LibraryAction, LibraryChanged,
    LibraryFields, NavigationAction, NavigationChanged, Notice, PlaybackAction, PlaybackChanged,
    PlaybackFields, PlaylistUpdated, QueueChanged, Route, SavedChanged, SearchAction,
    SearchChanged, SelectionAction, SelectionChanged, SessionAction, SessionChanged,
    SettingsChanged, Transition,
};
use riff_core::remote::Failure;
pub use riff_core::state::{Action, AppState};

#[derive(Clone, Debug)]
pub enum AppAction {
    Core(Action),
    Start,
    Raise,
    ShowNotification(String),
    ShowRestartRequired,
    ShowDrmBlockedDialog,
    ChangeCardLayout(CardLayout),
    ChangeCardSize(CardSize),
    ChangeSortOrder(String, SortOrder),
    PlaySong(String),
    PageLoaded(Route, PageFields),
}

impl AppAction {
    #[allow(non_snake_case)]
    pub fn OpenURI(uri: String) -> Option<Self> {
        debug!("parsing {}", &uri);
        match riff_api::parse_link(&uri)? {
            Link::Album(id) => Some(Self::ViewAlbum(id)),
            Link::Artist(id) => Some(Self::ViewArtist(id)),
            Link::Playlist(id) => Some(Self::ViewPlaylist(id)),
            Link::User(id) => Some(Self::ViewUser(id)),
            Link::Track(_) => None,
        }
    }

    #[allow(non_snake_case)]
    pub fn ViewAlbum(id: String) -> Self {
        NavigationAction::Go(Route::Album(id)).into()
    }

    #[allow(non_snake_case)]
    pub fn ViewArtist(id: String) -> Self {
        NavigationAction::Go(Route::Artist(id)).into()
    }

    #[allow(non_snake_case)]
    pub fn ViewPlaylist(id: String) -> Self {
        NavigationAction::Go(Route::Playlist(id)).into()
    }

    #[allow(non_snake_case)]
    pub fn ViewUser(id: String) -> Self {
        NavigationAction::Go(Route::User(id)).into()
    }

    #[allow(non_snake_case)]
    pub fn ViewSearch() -> Self {
        NavigationAction::Go(Route::Search).into()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    Core(Event),
    ViewPreferences(ViewPreferencesChanged),
    PageLoaded(PageLoaded),
    SongPlaybackRequested(String),
    Started,
    Raised,
    NotificationShown(String),
    RestartRequiredShown,
}

macro_rules! impl_from_core_event {
    ($($ty:ty),* $(,)?) => {
        $(impl From<$ty> for AppEvent {
            fn from(event: $ty) -> Self {
                Self::Core(event.into())
            }
        })*
    };
}

impl_from_core_event!(
    Event,
    PlaybackChanged,
    QueueChanged,
    DevicesChanged,
    PlaylistUpdated,
    LibraryChanged,
    SearchChanged,
    SessionChanged,
    NavigationChanged,
    SelectionChanged,
    ErrorKind,
);

macro_rules! core_accessors {
    ($($name:ident: $variant:ident => $ty:ty),* $(,)?) => {
        $(pub fn $name(&self) -> Option<&$ty> {
            match self {
                Self::Core(Event::$variant(event)) => Some(event),
                _ => None,
            }
        })*
    };
}

impl AppEvent {
    core_accessors!(
        playback: Playback => PlaybackChanged,
        devices: Devices => DevicesChanged,
        library: Library => LibraryChanged,
        search: Search => SearchChanged,
        session: Session => SessionChanged,
        navigation: Navigation => NavigationChanged,
        settings: Settings => SettingsChanged,
    );

    pub fn core(&self) -> Option<&Event> {
        match self {
            Self::Core(event) => Some(event),
            _ => None,
        }
    }

    pub fn transition(&self) -> Option<&Transition> {
        self.navigation()?.transition.as_ref()
    }

    pub fn queue_changed(&self) -> bool {
        matches!(self, Self::Core(Event::Queue(_)))
    }

    pub fn playback_changed(&self, fields: PlaybackFields) -> bool {
        self.playback()
            .is_some_and(|playback| playback.changed.intersects(fields))
    }

    pub fn library_changed(&self, fields: LibraryFields) -> bool {
        self.library()
            .is_some_and(|library| library.changed.intersects(fields))
    }

    pub fn page_loaded(&self, route: &Route) -> Option<PageFields> {
        match self {
            Self::PageLoaded(loaded) if loaded.route == *route => Some(loaded.changed),
            _ => None,
        }
    }

    pub fn saved(&self, item: &ItemId) -> Option<bool> {
        match self {
            Self::Core(Event::Saved(SavedChanged {
                item: changed,
                saved,
            })) if changed == item => Some(*saved),
            _ => None,
        }
    }

    pub fn selection(&self) -> Option<&SelectionChanged> {
        match self {
            Self::Core(Event::Selection(event)) => Some(event),
            _ => None,
        }
    }
}

pub fn app_event(event: Event) -> AppEvent {
    match event {
        Event::Notice(Notice::AddedToQueue) => {
            AppEvent::NotificationShown(labels::ADDED_TO_QUEUE.clone())
        }
        Event::Error(ErrorKind::Device(failure)) => {
            AppEvent::NotificationShown(device_failure_label(failure))
        }
        event => AppEvent::Core(event),
    }
}

fn device_failure_label(failure: Failure) -> String {
    match failure {
        // translators: Shown when a playback control can't reach the account's other devices.
        Failure::Offline => gettext("Not connected to your other devices"),
        // translators: Shown when the device playing didn't respond to a playback control.
        Failure::Unreachable => gettext("The device didn't respond"),
        // translators: Shown when the device playing turned down a playback control, e.g. skipping where it can't.
        Failure::Refused => gettext("The device can't do that right now"),
        // translators: Shown when playback can't move to this app yet, as other devices can't see it.
        Failure::ThisDeviceUnlisted => gettext("This device isn't ready to play yet"),
    }
}

pub fn ui_events(action: AppAction, started: &mut bool) -> Vec<AppEvent> {
    match action {
        AppAction::Core(_) => unreachable!("core actions change the core state"),
        AppAction::Start if !*started => {
            *started = true;
            vec![AppEvent::Started]
        }
        AppAction::Start => vec![],
        AppAction::Raise => vec![AppEvent::Raised],
        AppAction::ShowNotification(content) => vec![AppEvent::NotificationShown(content)],
        AppAction::ShowRestartRequired => vec![AppEvent::RestartRequiredShown],
        AppAction::ShowDrmBlockedDialog => vec![ErrorKind::DrmBlocked.into()],
        AppAction::ChangeCardLayout(layout) => {
            vec![ViewPreferencesChanged::CardLayout(layout).into()]
        }
        AppAction::ChangeCardSize(size) => vec![ViewPreferencesChanged::CardSize(size).into()],
        AppAction::ChangeSortOrder(page, order) => {
            vec![ViewPreferencesChanged::SortOrder(page, order).into()]
        }
        AppAction::PlaySong(id) => vec![AppEvent::SongPlaybackRequested(id)],
        AppAction::PageLoaded(route, changed) => vec![PageLoaded { route, changed }.into()],
    }
}

impl From<Action> for AppAction {
    fn from(action: Action) -> Self {
        Self::Core(action)
    }
}

macro_rules! impl_from_core_action {
    ($($ty:ty),* $(,)?) => {
        $(impl From<$ty> for AppAction {
            fn from(action: $ty) -> Self {
                Self::Core(action.into())
            }
        })*
    };
}

impl_from_core_action!(
    PlaybackAction,
    BrowserAction,
    NavigationAction,
    LibraryAction,
    SearchAction,
    SelectionAction,
    SessionAction,
);

pub fn playback_actions(actions: Vec<PlaybackAction>) -> Vec<AppAction> {
    actions.into_iter().map(AppAction::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pages_announce_what_they_loaded() {
        let route = Route::Album("a".into());
        let events = ui_events(
            AppAction::PageLoaded(route.clone(), PageFields::TRACKS),
            &mut true,
        );
        assert_eq!(events[0].page_loaded(&route), Some(PageFields::TRACKS));
        assert_eq!(events[0].page_loaded(&Route::Album("b".into())), None);
    }

    #[test]
    fn test_notices_become_notifications() {
        assert!(matches!(
            app_event(Notice::AddedToQueue.into()),
            AppEvent::NotificationShown(_)
        ));
        assert_eq!(
            app_event(QueueChanged.into()),
            AppEvent::Core(QueueChanged.into())
        );
    }
}
