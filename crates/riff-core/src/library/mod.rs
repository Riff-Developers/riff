mod pins;
mod requests;

use riff_api::models::{is_resource_url, ImageSet, Playlist};

use crate::browse::LibraryAction;
use crate::events::{
    Event, ItemId, LibraryChanged, LibraryFields, PlaylistFields, PlaylistUpdated, SavedChanged,
};

pub use pins::{Pin, PinKind, Pins};
pub use requests::{
    add_tracks_to_playlist, create_playlist, edit_playlist, follow_playlist, load_saved_playlists,
    refreshed_playlist_cover, remove_from_playlist, rename_playlist, set_saved,
    upload_playlist_cover, PlaylistDetails, TracksAdded,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistSummary {
    pub id: String,
    pub title: String,
}

impl From<Playlist> for PlaylistSummary {
    fn from(playlist: Playlist) -> Self {
        Self {
            id: playlist.rri.id,
            title: playlist.title,
        }
    }
}

#[derive(Debug, Default)]
pub struct Library {
    playlists: Option<Vec<Playlist>>,
    pins: Pins,
}

fn emit(changed: LibraryFields) -> Vec<Event> {
    vec![LibraryChanged { changed }.into()]
}

fn emit_if(any: bool, changed: LibraryFields) -> Vec<Event> {
    if any {
        emit(changed)
    } else {
        vec![]
    }
}

impl Library {
    pub fn apply(&mut self, action: &LibraryAction) -> Vec<Event> {
        match action {
            LibraryAction::PlaylistsLoaded(playlists) => {
                let mut events = self.set_playlists(playlists.clone());
                events.extend(emit_if(self.prune_playlist_pins(), LibraryFields::PINS));
                events
            }
            LibraryAction::SavePlaylist(playlist) => {
                let mut events = self.add_playlist((**playlist).clone());
                events.push(saved(ItemId::Playlist(playlist.rri.id.clone()), true));
                events
            }
            LibraryAction::SetSaved(item, is_saved) => {
                let mut events = match (item, is_saved) {
                    (ItemId::Playlist(id), false) => self.remove_playlist(id),
                    _ => vec![],
                };
                if !is_saved {
                    let unpinned = pins::pins_of(item)
                        .into_iter()
                        .fold(false, |any, (kind, id)| self.pins.remove(kind, id) || any);
                    events.extend(emit_if(unpinned, LibraryFields::PINS));
                }
                events.push(saved(item.clone(), *is_saved));
                events
            }
            LibraryAction::RenamePlaylist(PlaylistSummary { id, title }) => {
                let mut events = self.rename_playlist(id, title);
                events.push(PlaylistUpdated::new(id, PlaylistFields::INFO).into());
                events
            }
            LibraryAction::SetPlaylistArt(id, art) => {
                let mut events = self.set_playlist_art(id, art);
                events.push(PlaylistUpdated::new(id, PlaylistFields::INFO).into());
                events
            }
            LibraryAction::PlaylistEdited(id, changed) => {
                vec![PlaylistUpdated::new(id, *changed).into()]
            }
            LibraryAction::PinsLoaded(pins) => {
                let changed = self.pins.set(pins.clone()) | self.prune_playlist_pins();
                emit_if(changed, LibraryFields::PINS)
            }
            LibraryAction::Pin(pin) => {
                let saved = pin.kind != PinKind::Playlist || self.has_playlist(&pin.id);
                emit_if(saved && self.pins.add(pin.clone()), LibraryFields::PINS)
            }
            LibraryAction::Unpin(kind, id) => {
                emit_if(self.pins.remove(*kind, id), LibraryFields::PINS)
            }
        }
    }

    pub fn pins(&self) -> &Pins {
        &self.pins
    }

    pub fn is_pinned(&self, kind: PinKind, id: &str) -> bool {
        self.pins.contains(kind, id)
    }

    pub fn pin_title(&self, pin: &Pin) -> Option<String> {
        match pin.kind {
            PinKind::Playlist => self
                .playlist(&pin.id)
                .map(|p| p.title.clone())
                .or_else(|| pin.title.clone()),
            _ => pin.title.clone(),
        }
    }

    pub fn unpinned_playlists(&self) -> impl Iterator<Item = &Playlist> {
        self.playlists()
            .iter()
            .filter(|p| !self.pins.contains(PinKind::Playlist, &p.rri.id))
    }

    fn prune_playlist_pins(&mut self) -> bool {
        let Some(playlists) = self.playlists.as_ref() else {
            return false;
        };
        let saved = |id: &str| playlists.iter().any(|p| p.rri.id == id);
        self.pins.retain_kind(PinKind::Playlist, saved)
    }

    pub fn playlists(&self) -> &[Playlist] {
        self.playlists.as_deref().unwrap_or_default()
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists().iter().find(|p| p.rri.id == id)
    }

    pub fn playlist_lacks_cover(&self, id: &str) -> bool {
        self.playlist(id)
            .is_some_and(|p| p.art.largest().is_none_or(is_resource_url))
    }

    pub fn has_playlist(&self, id: &str) -> bool {
        self.playlist(id).is_some()
    }

    pub fn is_playlist_owned_by(&self, id: &str, user: &str) -> bool {
        self.playlist(id).is_some_and(|p| is_owned_by(p, user))
    }

    pub fn owned_playlists<'a>(&'a self, user: &'a str) -> impl Iterator<Item = &'a Playlist> {
        self.playlists()
            .iter()
            .filter(move |p| is_owned_by(p, user))
    }

    fn set_playlists(&mut self, playlists: Vec<Playlist>) -> Vec<Event> {
        let same = self.playlists.as_ref().is_some_and(|current| {
            current.len() == playlists.len()
                && current.iter().zip(&playlists).all(|(a, b)| same_card(a, b))
        });
        self.playlists = Some(playlists);
        emit_if(!same, LibraryFields::PLAYLISTS)
    }

    fn add_playlist(&mut self, playlist: Playlist) -> Vec<Event> {
        let playlists = self.playlists.get_or_insert_with(Vec::new);
        if playlists.iter().any(|p| p.rri.id == playlist.rri.id) {
            return vec![];
        }
        playlists.insert(0, playlist);
        emit(LibraryFields::PLAYLISTS)
    }

    fn remove_playlist(&mut self, id: &str) -> Vec<Event> {
        let Some(playlists) = self.playlists.as_mut() else {
            return vec![];
        };
        let before = playlists.len();
        playlists.retain(|p| p.rri.id != id);
        emit_if(playlists.len() != before, LibraryFields::PLAYLISTS)
    }

    fn rename_playlist(&mut self, id: &str, title: &str) -> Vec<Event> {
        self.update_playlist(id, |p| {
            let renamed = p.title != title;
            p.title = title.to_string();
            renamed
        })
    }

    fn set_playlist_art(&mut self, id: &str, art: &ImageSet) -> Vec<Event> {
        self.update_playlist(id, |p| {
            let changed = &p.art != art;
            p.art = art.clone();
            changed
        })
    }

    fn update_playlist(
        &mut self,
        id: &str,
        change: impl FnOnce(&mut Playlist) -> bool,
    ) -> Vec<Event> {
        let playlist = self
            .playlists
            .as_mut()
            .and_then(|playlists| playlists.iter_mut().find(|p| p.rri.id == id));
        emit_if(playlist.is_some_and(change), LibraryFields::PLAYLISTS)
    }

    pub fn clear(&mut self) -> Vec<Event> {
        *self = Self::default();
        emit(LibraryFields::PLAYLISTS | LibraryFields::PINS)
    }
}

fn saved(item: ItemId, saved: bool) -> Event {
    SavedChanged { item, saved }.into()
}

fn is_owned_by(playlist: &Playlist, user: &str) -> bool {
    playlist.owner.as_ref().is_some_and(|o| o.rri.id == user)
}

fn same_card(a: &Playlist, b: &Playlist) -> bool {
    a.rri.id == b.rri.id && a.title == b.title && a.art == b.art
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::playlist;

    fn owned(id: &str, owner: &str) -> Playlist {
        let mut playlist = playlist(id);
        playlist.owner = Some(riff_api::models::UserRef {
            rri: crate::test_support::rri(owner),
            display_name: owner.into(),
        });
        playlist
    }

    fn ids(library: &Library) -> Vec<&str> {
        library
            .playlists()
            .iter()
            .map(|p| p.rri.id.as_str())
            .collect()
    }

    #[test]
    fn saved_playlists_change_in_place() {
        let mut library = Library::default();
        assert!(library.playlists.is_none());
        let load = || LibraryAction::PlaylistsLoaded(vec![owned("a", "me"), owned("b", "you")]);
        assert_eq!(library.apply(&load()).len(), 1);
        assert!(library.apply(&load()).is_empty());

        let events = library.apply(&LibraryAction::SavePlaylist(Box::new(owned("c", "me"))));
        assert!(events.contains(&saved(ItemId::Playlist("c".into()), true)));
        assert_eq!(ids(&library), ["c", "a", "b"]);

        library.apply(&LibraryAction::RenamePlaylist(PlaylistSummary {
            id: "a".into(),
            title: "New".into(),
        }));
        assert_eq!(library.playlist("a").unwrap().title, "New");

        library.apply(&LibraryAction::SetSaved(
            ItemId::Playlist("b".into()),
            false,
        ));
        assert_eq!(ids(&library), ["c", "a"]);
        assert!(library.is_playlist_owned_by("c", "me"));
        assert!(!library.is_playlist_owned_by("c", "you"));
    }

    #[test]
    fn other_saves_are_only_announced() {
        let mut library = Library::default();
        let item = ItemId::Tracks(vec!["t".into()]);
        let events = library.apply(&LibraryAction::SetSaved(item.clone(), true));
        assert_eq!(events, [saved(item, true)]);
    }

    #[test]
    fn pins_follow_what_is_saved() {
        let mut library = Library::default();
        let album = Pin::new(PinKind::Album, "al", Some("Album".into()));
        let playlist = Pin::new(PinKind::Playlist, "p", Some("Old".into()));
        let gone = Pin::new(PinKind::Playlist, "gone", None);
        library.apply(&LibraryAction::PinsLoaded(vec![
            album.clone(),
            playlist.clone(),
            gone.clone(),
        ]));
        assert!(library.pins().is_loaded());
        assert!(
            library.is_pinned(PinKind::Playlist, "gone"),
            "playlists not loaded yet"
        );

        let mut renamed = owned("p", "me");
        renamed.title = "New".into();
        let events = library.apply(&LibraryAction::PlaylistsLoaded(vec![
            renamed,
            owned("q", "me"),
        ]));
        assert!(events.contains(
            &LibraryChanged {
                changed: LibraryFields::PINS
            }
            .into()
        ));
        assert!(!library.is_pinned(PinKind::Playlist, "gone"));
        assert_eq!(library.pin_title(&playlist).as_deref(), Some("New"));
        assert_eq!(library.pin_title(&album).as_deref(), Some("Album"));
        let unpinned: Vec<_> = library
            .unpinned_playlists()
            .map(|p| p.rri.id.as_str())
            .collect();
        assert_eq!(unpinned, ["q"]);

        assert!(library.apply(&LibraryAction::Pin(gone)).is_empty());
        assert_eq!(
            library
                .apply(&LibraryAction::Pin(Pin::new(PinKind::Playlist, "q", None)))
                .len(),
            1
        );

        library.apply(&LibraryAction::SetSaved(ItemId::Album("al".into()), false));
        assert!(!library.is_pinned(PinKind::Album, "al"));
        library.apply(&LibraryAction::Unpin(PinKind::Playlist, "q".into()));
        assert!(!library.is_pinned(PinKind::Playlist, "q"));
    }
}
