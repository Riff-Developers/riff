use std::sync::Arc;

use riff_api::models::{is_resource_url, ImageSet, Page, Playlist, Track};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, LikedTracks, PageFields, Screen, TrackPages, Update};
use crate::browse::LibraryAction;
use crate::events::{Event, PlaylistFields};
use crate::library;
use crate::navigation::Route;
use crate::pagination::PAGE_SIZE;

pub struct PlaylistScreen {
    id: String,
    pub playlist: Option<Playlist>,
    pub tracks: TrackPages,
    liked: LikedTracks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistRequest {
    Info,
    Tracks(usize),
    Cover,
}

pub enum PlaylistResponse {
    Info(Box<Playlist>),
    Tracks(Page<Track>),
    Cover(Option<ImageSet>),
}

impl PlaylistScreen {
    pub fn new(id: String, liked: LikedTracks) -> Self {
        Self {
            id,
            playlist: None,
            tracks: TrackPages::default(),
            liked,
        }
    }

    fn lacks_cover(playlist: &Playlist) -> bool {
        playlist
            .art
            .images
            .first()
            .is_none_or(|image| is_resource_url(&image.url))
    }
}

impl Screen for PlaylistScreen {
    type Request = PlaylistRequest;
    type Response = PlaylistResponse;

    fn route(&self) -> Route {
        Route::Playlist(self.id.clone())
    }

    fn open(&mut self) -> Vec<PlaylistRequest> {
        vec![
            PlaylistRequest::Info,
            PlaylistRequest::Tracks(self.tracks.restart()),
        ]
    }

    fn load_more(&mut self) -> Vec<PlaylistRequest> {
        self.tracks.next_request(PlaylistRequest::Tracks)
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        request: PlaylistRequest,
        load: Load,
    ) -> Fetch<PlaylistResponse> {
        let id = self.id.clone();
        Box::pin(async move {
            Ok(match request {
                PlaylistRequest::Info => {
                    PlaylistResponse::Info(Box::new(api.get_playlist(&id, load).await?))
                }
                PlaylistRequest::Tracks(offset) => PlaylistResponse::Tracks(
                    api.get_playlist_tracks(&id, offset, PAGE_SIZE, load)
                        .await?,
                ),
                PlaylistRequest::Cover => PlaylistResponse::Cover(
                    library::refreshed_playlist_cover(&api, &id, false).await,
                ),
            })
        })
    }

    fn receive(
        &mut self,
        request: PlaylistRequest,
        result: Result<PlaylistResponse, DomainError>,
    ) -> Update<PlaylistRequest> {
        match (request, result) {
            (PlaylistRequest::Info, Ok(PlaylistResponse::Info(playlist))) => {
                let cover = Self::lacks_cover(&playlist).then_some(PlaylistRequest::Cover);
                let title = playlist.title.clone();
                self.playlist = Some(*playlist);
                Update::changed(PageFields::INFO)
                    .titled(title)
                    .and_load(cover.into_iter().collect())
            }
            (PlaylistRequest::Info, Err(error)) => Update::info_failed(self.route(), error),
            (PlaylistRequest::Tracks(offset), Ok(PlaylistResponse::Tracks(page))) => {
                self.liked.learn(&page.items);
                self.tracks.loaded(offset, page, PageFields::TRACKS)
            }
            (PlaylistRequest::Tracks(offset), Err(error)) => self.tracks.failed(offset, error),
            (PlaylistRequest::Cover, Ok(PlaylistResponse::Cover(Some(art)))) => Update {
                actions: vec![LibraryAction::SetPlaylistArt(self.id.clone(), art).into()],
                ..Default::default()
            },
            _ => Update::default(),
        }
    }

    fn apply(&mut self, event: &Event) -> Update<PlaylistRequest> {
        let Event::Playlist(edited) = event else {
            return Update::default();
        };
        if edited.id != self.id {
            return Update::default();
        }
        let mut requests = vec![];
        if edited.changed.contains(PlaylistFields::INFO) {
            requests.push(PlaylistRequest::Info);
        }
        if edited.changed.contains(PlaylistFields::TRACKS) {
            requests.push(PlaylistRequest::Tracks(self.tracks.restart()));
        }
        Update::load(requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::BrowserAction;
    use crate::events::PlaylistUpdated;
    use crate::test_support::{page, playlist, tracks};

    fn screen() -> PlaylistScreen {
        PlaylistScreen::new("p".into(), LikedTracks::default())
    }

    #[test]
    fn a_playlist_without_a_cover_waits_for_one() {
        let mut screen = screen();
        screen.open();
        let update = screen.receive(
            PlaylistRequest::Info,
            Ok(PlaylistResponse::Info(Box::new(playlist("p")))),
        );
        assert_eq!(update.changed, PageFields::INFO);
        assert_eq!(update.title.as_deref(), Some("p"));
        assert_eq!(update.requests, [PlaylistRequest::Cover]);

        let update = screen.receive(
            PlaylistRequest::Cover,
            Ok(PlaylistResponse::Cover(Some(ImageSet::default()))),
        );
        assert!(matches!(
            update.actions[..],
            [BrowserAction::Library(LibraryAction::SetPlaylistArt(..))]
        ));
    }

    #[test]
    fn edits_reload_what_changed() {
        let mut screen = screen();
        screen.open();
        screen.receive(
            PlaylistRequest::Tracks(0),
            Ok(PlaylistResponse::Tracks(page(tracks("t", 0..50), 0))),
        );
        assert_eq!(screen.load_more(), [PlaylistRequest::Tracks(50)]);

        let edited = |changed| Event::from(PlaylistUpdated::new("p", changed));
        let update = screen.apply(&edited(PlaylistFields::TRACKS));
        assert_eq!(update.requests, [PlaylistRequest::Tracks(0)]);
        assert!(screen.load_more().is_empty(), "waits for the first page");

        let update = screen.apply(&edited(PlaylistFields::INFO));
        assert_eq!(update.requests, [PlaylistRequest::Info]);
        let other = Event::from(PlaylistUpdated::new("q", PlaylistFields::INFO));
        assert!(screen.apply(&other).requests.is_empty());
    }

    #[test]
    fn a_missing_playlist_is_left() {
        let mut screen = screen();
        let update = screen.receive(
            PlaylistRequest::Info,
            Err(DomainError::NotFound {
                resource: "p".into(),
            }),
        );
        assert!(update.error.is_none());
        assert!(matches!(update.actions[..], [BrowserAction::Navigation(_)]));

        let update = screen.receive(PlaylistRequest::Info, Err(DomainError::Shed));
        assert!(update.actions.is_empty() && update.error.is_some());
    }
}
