use std::sync::Arc;

use riff_api::models::{Album, Artist, Page, Track};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, LikedTracks, PageFields, Pages, Screen, Update};
use crate::events::{Event, ItemId, SavedChanged};
use crate::navigation::Route;
use crate::pagination::PAGE_SIZE;

const TOP_TRACKS: usize = 10;

pub struct ArtistScreen {
    id: String,
    pub artist: Option<Artist>,
    pub top_tracks: Vec<Track>,
    pub releases: Pages<Album>,
    pub followed: bool,
    liked: LikedTracks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtistRequest {
    Info,
    TopTracks,
    Releases(usize),
    Followed,
}

pub enum ArtistResponse {
    Info(Artist),
    TopTracks(Vec<Track>),
    Releases(Page<Album>),
    Followed(bool),
}

impl ArtistScreen {
    pub fn new(id: String, liked: LikedTracks) -> Self {
        Self {
            id,
            artist: None,
            top_tracks: vec![],
            releases: Pages::default(),
            followed: false,
            liked,
        }
    }
}

impl Screen for ArtistScreen {
    type Request = ArtistRequest;
    type Response = ArtistResponse;

    fn route(&self) -> Route {
        Route::Artist(self.id.clone())
    }

    fn open(&mut self) -> Vec<ArtistRequest> {
        vec![
            ArtistRequest::Info,
            ArtistRequest::Releases(self.releases.restart()),
            ArtistRequest::TopTracks,
            ArtistRequest::Followed,
        ]
    }

    fn load_more(&mut self) -> Vec<ArtistRequest> {
        self.releases.next_request(ArtistRequest::Releases)
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        request: ArtistRequest,
        load: Load,
    ) -> Fetch<ArtistResponse> {
        let id = self.id.clone();
        Box::pin(async move {
            Ok(match request {
                ArtistRequest::Info => ArtistResponse::Info(api.get_artist(&id, load).await?),
                ArtistRequest::TopTracks => {
                    ArtistResponse::TopTracks(api.get_artist_top_tracks(&id, load).await?)
                }
                ArtistRequest::Releases(offset) => ArtistResponse::Releases(
                    api.get_artist_albums(&id, offset, PAGE_SIZE, load).await?,
                ),
                ArtistRequest::Followed => {
                    ArtistResponse::Followed(api.check_following_artist(&id, load).await?)
                }
            })
        })
    }

    fn receive(
        &mut self,
        request: ArtistRequest,
        result: Result<ArtistResponse, DomainError>,
    ) -> Update<ArtistRequest> {
        match (request, result) {
            (ArtistRequest::Info, Ok(ArtistResponse::Info(artist))) => {
                let title = artist.name.clone();
                self.artist = Some(artist);
                Update::changed(PageFields::INFO).titled(title)
            }
            (ArtistRequest::Info, Err(error)) => Update::info_failed(self.route(), error),
            (ArtistRequest::TopTracks, Ok(ArtistResponse::TopTracks(mut tracks))) => {
                tracks.truncate(TOP_TRACKS);
                self.liked.learn(&tracks);
                self.top_tracks = tracks;
                Update::changed(PageFields::TRACKS)
            }
            (ArtistRequest::Releases(offset), Ok(ArtistResponse::Releases(page))) => {
                self.releases.loaded(offset, page, PageFields::CARDS)
            }
            (ArtistRequest::Releases(offset), Err(error)) => self.releases.failed(offset, error),
            (ArtistRequest::Followed, result) => {
                self.followed = matches!(result, Ok(ArtistResponse::Followed(true)));
                Update::changed(PageFields::SAVED)
            }
            (_, Err(error)) => Update::error(error),
            _ => Update::default(),
        }
    }

    fn apply(&mut self, event: &Event) -> Update<ArtistRequest> {
        match event {
            Event::Saved(SavedChanged {
                item: ItemId::Artist(id),
                saved,
            }) if *id == self.id => {
                self.followed = *saved;
                Update::changed(PageFields::SAVED)
            }
            _ => Update::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{page, tracks};

    #[test]
    fn top_tracks_and_releases() {
        let liked = LikedTracks::default();
        let mut screen = ArtistScreen::new("a".into(), liked.clone());
        assert_eq!(screen.open().len(), 4);

        let mut top = tracks("t", 0..12);
        top[0].saved = Some(true);
        let update = screen.receive(ArtistRequest::TopTracks, Ok(ArtistResponse::TopTracks(top)));
        assert_eq!(update.changed, PageFields::TRACKS);
        assert_eq!(screen.top_tracks.len(), TOP_TRACKS);
        assert!(liked.is_liked("t0"));

        screen.receive(
            ArtistRequest::Releases(0),
            Ok(ArtistResponse::Releases(page(vec![], 0))),
        );
        assert!(screen.load_more().is_empty(), "no more releases");

        screen.receive(ArtistRequest::Followed, Err(DomainError::Shed));
        assert!(!screen.followed);
        let followed = Event::Saved(SavedChanged {
            item: ItemId::Artist("a".into()),
            saved: true,
        });
        screen.apply(&followed);
        assert!(screen.followed);
    }
}
