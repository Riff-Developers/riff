use std::collections::HashSet;
use std::sync::Arc;

use riff_api::models::{Album, Artist, Page, Track};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, LikedTracks, PageFields, Pages, Screen, TrackPages, Update};
use crate::events::{Event, ItemId, SavedChanged};
use crate::navigation::{HomePage, Route};
use crate::pagination::PAGE_SIZE;

const ARTISTS_PAGE: usize = 30;

pub struct SavedTracksScreen {
    pub tracks: TrackPages,
    liked: LikedTracks,
}

impl SavedTracksScreen {
    pub fn new(liked: LikedTracks) -> Self {
        Self {
            tracks: TrackPages::default(),
            liked,
        }
    }

    pub fn count(&self) -> usize {
        let loaded = self.tracks.items.len();
        self.tracks.total.map_or(loaded, |total| total.max(loaded))
    }
}

impl Screen for SavedTracksScreen {
    type Request = usize;
    type Response = Page<Track>;

    fn route(&self) -> Route {
        Route::Home(HomePage::SavedTracks)
    }

    fn open(&mut self) -> Vec<usize> {
        vec![self.tracks.restart()]
    }

    fn load_more(&mut self) -> Vec<usize> {
        self.tracks.next_offset().into_iter().collect()
    }

    fn fetch(&self, api: Arc<ApiService>, offset: usize, load: Load) -> Fetch<Page<Track>> {
        Box::pin(async move { api.get_saved_tracks(offset, PAGE_SIZE, load).await })
    }

    fn receive(
        &mut self,
        offset: usize,
        result: Result<Page<Track>, DomainError>,
    ) -> Update<usize> {
        match result {
            Ok(page) => {
                self.liked.learn_liked(&page.items);
                self.tracks.loaded(offset, page, PageFields::TRACKS)
            }
            Err(error) => self.tracks.failed(offset, error),
        }
    }

    fn apply(&mut self, event: &Event) -> Update<usize> {
        match event {
            Event::Saved(SavedChanged {
                item: ItemId::Tracks(_),
                saved: true,
            }) => Update::load(self.open()),
            Event::Saved(SavedChanged {
                item: ItemId::Tracks(ids),
                saved: false,
            }) => {
                self.tracks.remove(|track| ids.contains(&track.rri.id));
                Update::changed(PageFields::TRACKS)
            }
            _ => Update::default(),
        }
    }
}

#[derive(Default)]
pub struct SavedAlbumsScreen {
    pub albums: Pages<Album>,
}

impl Screen for SavedAlbumsScreen {
    type Request = usize;
    type Response = Page<Album>;

    fn route(&self) -> Route {
        Route::Home(HomePage::Library)
    }

    fn open(&mut self) -> Vec<usize> {
        vec![self.albums.restart()]
    }

    fn load_more(&mut self) -> Vec<usize> {
        self.albums.next_offset().into_iter().collect()
    }

    fn fetch(&self, api: Arc<ApiService>, offset: usize, load: Load) -> Fetch<Page<Album>> {
        Box::pin(async move { api.get_saved_albums(offset, PAGE_SIZE, load).await })
    }

    fn receive(
        &mut self,
        offset: usize,
        result: Result<Page<Album>, DomainError>,
    ) -> Update<usize> {
        match result {
            Ok(page) => {
                self.albums.load(offset, page);
                let mut seen = HashSet::new();
                self.albums
                    .items
                    .retain(|album| seen.insert(album.rri.id.clone()));
                Update::changed(PageFields::CARDS)
            }
            Err(error) => self.albums.failed(offset, error),
        }
    }

    fn apply(&mut self, event: &Event) -> Update<usize> {
        match event {
            Event::Saved(SavedChanged {
                item: ItemId::Album(_),
                ..
            }) => Update::load(self.open()),
            _ => Update::default(),
        }
    }
}

#[derive(Default)]
pub struct SavedArtistsScreen {
    pub artists: Vec<Artist>,
    cursor: Option<String>,
}

pub type SavedArtistsRequest = Option<String>;

impl SavedArtistsScreen {
    pub fn has_more(&self) -> bool {
        self.cursor.as_ref().is_some_and(|c| !c.is_empty())
    }
}

impl Screen for SavedArtistsScreen {
    type Request = SavedArtistsRequest;
    type Response = (Vec<Artist>, Option<String>);

    fn route(&self) -> Route {
        Route::Home(HomePage::SavedArtists)
    }

    fn open(&mut self) -> Vec<SavedArtistsRequest> {
        self.cursor = None;
        vec![None]
    }

    fn load_more(&mut self) -> Vec<SavedArtistsRequest> {
        if !self.has_more() {
            return vec![];
        }
        vec![self.cursor.take()]
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        after: SavedArtistsRequest,
        load: Load,
    ) -> Fetch<Self::Response> {
        Box::pin(async move {
            api.get_followed_artists(after.as_deref(), ARTISTS_PAGE, load)
                .await
        })
    }

    fn receive(
        &mut self,
        after: SavedArtistsRequest,
        result: Result<Self::Response, DomainError>,
    ) -> Update<SavedArtistsRequest> {
        match result {
            Ok((artists, cursor)) => {
                if after.is_none() {
                    self.artists = artists;
                } else {
                    self.artists.extend(artists);
                }
                self.cursor = cursor;
                Update::changed(PageFields::CARDS)
            }
            Err(error) => {
                self.cursor = after;
                Update::error(error)
            }
        }
    }

    fn apply(&mut self, event: &Event) -> Update<SavedArtistsRequest> {
        match event {
            Event::Saved(SavedChanged {
                item: ItemId::Artist(_),
                ..
            }) => Update::load(self.open()),
            _ => Update::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{album, page, rri, saved_tracks, tracks};

    #[test]
    fn saved_tracks_follow_likes() {
        let liked = LikedTracks::default();
        let mut screen = SavedTracksScreen::new(liked.clone());
        let offset = screen.open()[0];
        let mut first = page(tracks("t", 0..3), offset);
        first.total = Some(3);
        screen.receive(offset, Ok(first));
        assert!(liked.is_liked("t1"));
        assert_eq!(screen.count(), 3);

        let update = screen.apply(&saved_tracks(&["t1"], false));
        assert_eq!(update.changed, PageFields::TRACKS);
        assert_eq!(screen.count(), 2);

        let update = screen.apply(&saved_tracks(&["n"], true));
        assert_eq!(update.requests, [0], "loads the list again");
    }

    #[test]
    fn saved_albums_keep_one_copy_of_each() {
        let mut screen = SavedAlbumsScreen::default();
        screen.open();
        let full: Vec<Album> = (0..PAGE_SIZE).map(|i| album(&i.to_string())).collect();
        screen.receive(0, Ok(page(full, 0)));
        let next = screen.load_more()[0];
        let last = (PAGE_SIZE - 1).to_string();
        screen.receive(next, Ok(page(vec![album(&last), album("new")], next)));
        assert_eq!(screen.albums.items.len(), PAGE_SIZE + 1);
    }

    #[test]
    fn saved_artists_go_by_cursor() {
        let artist = |id: &str| Artist {
            rri: rri(id),
            name: id.into(),
            ..Default::default()
        };
        let mut screen = SavedArtistsScreen::default();
        assert_eq!(screen.open(), [None]);
        screen.receive(None, Ok((vec![artist("a")], Some("c1".into()))));
        assert!(screen.has_more());
        let next = screen.load_more();
        assert_eq!(next, [Some("c1".to_string())]);
        assert!(screen.load_more().is_empty(), "already loading");

        screen.receive(next[0].clone(), Err(DomainError::Shed));
        assert_eq!(screen.load_more(), [Some("c1".to_string())]);
        screen.receive(Some("c1".into()), Ok((vec![artist("b")], None)));
        assert_eq!(screen.artists.len(), 2);
        assert!(!screen.has_more());
    }
}
