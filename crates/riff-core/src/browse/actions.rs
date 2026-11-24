use riff_api::models::{ImageSet, Playlist, SearchType};

use crate::events::{ItemId, PlaylistFields};
use crate::library::{Pin, PinKind, PlaylistSummary};
use crate::navigation::{EntryId, Route};

#[derive(Clone, Debug)]
pub enum BrowserAction {
    Navigation(NavigationAction),
    Search(SearchAction),
    Library(LibraryAction),
}

#[derive(Clone, Debug)]
pub enum NavigationAction {
    Go(Route),
    Back,
    Forward,
    GoTo(EntryId),
    Leave(Route),
    SaveScroll(EntryId, u32),
}

#[derive(Clone, Debug)]
pub enum SearchAction {
    Query(String),
    Filter(Option<SearchType>),
}

#[derive(Clone, Debug)]
pub enum LibraryAction {
    PlaylistsLoaded(Vec<Playlist>),
    SavePlaylist(Box<Playlist>),
    SetSaved(ItemId, bool),
    RenamePlaylist(PlaylistSummary),
    SetPlaylistArt(String, ImageSet),
    PlaylistEdited(String, PlaylistFields),
    PinsLoaded(Vec<Pin>),
    Pin(Pin),
    Unpin(PinKind, String),
}

impl_from!(BrowserAction {
    NavigationAction => Navigation,
    SearchAction => Search,
    LibraryAction => Library,
});
