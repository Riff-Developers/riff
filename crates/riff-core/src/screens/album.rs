use std::sync::Arc;

use riff_api::models::{Album, Page, Track};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, LikedTracks, PageFields, Screen, TrackPages, Update};
use crate::events::{Event, ItemId, SavedChanged};
use crate::navigation::Route;
use crate::pagination::PAGE_SIZE;

pub struct AlbumScreen {
    id: String,
    pub album: Option<Album>,
    pub tracks: TrackPages,
    pub saved: bool,
    liked: LikedTracks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlbumRequest {
    Info,
    Tracks(usize),
    Saved,
}

pub enum AlbumResponse {
    Info(Box<Album>),
    Tracks(Page<Track>),
    Saved(bool),
}

impl AlbumScreen {
    pub fn new(id: String, liked: LikedTracks) -> Self {
        Self {
            id,
            album: None,
            tracks: TrackPages::default(),
            saved: false,
            liked,
        }
    }
}

impl Screen for AlbumScreen {
    type Request = AlbumRequest;
    type Response = AlbumResponse;

    fn route(&self) -> Route {
        Route::Album(self.id.clone())
    }

    fn open(&mut self) -> Vec<AlbumRequest> {
        vec![
            AlbumRequest::Info,
            AlbumRequest::Tracks(self.tracks.restart()),
            AlbumRequest::Saved,
        ]
    }

    fn load_more(&mut self) -> Vec<AlbumRequest> {
        self.tracks.next_request(AlbumRequest::Tracks)
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        request: AlbumRequest,
        load: Load,
    ) -> Fetch<AlbumResponse> {
        let id = self.id.clone();
        Box::pin(async move {
            Ok(match request {
                AlbumRequest::Info => {
                    AlbumResponse::Info(Box::new(api.get_album(&id, load).await?))
                }
                AlbumRequest::Tracks(offset) => {
                    AlbumResponse::Tracks(api.get_album_tracks(&id, offset, PAGE_SIZE, load).await?)
                }
                AlbumRequest::Saved => {
                    AlbumResponse::Saved(api.check_saved_album(&id, load).await?)
                }
            })
        })
    }

    fn receive(
        &mut self,
        request: AlbumRequest,
        result: Result<AlbumResponse, DomainError>,
    ) -> Update<AlbumRequest> {
        match (request, result) {
            (AlbumRequest::Info, Ok(AlbumResponse::Info(album))) => {
                let title = album.title.clone();
                self.album = Some(*album);
                Update::changed(PageFields::INFO).titled(title)
            }
            (AlbumRequest::Info, Err(error)) => Update::info_failed(self.route(), error),
            (AlbumRequest::Tracks(offset), Ok(AlbumResponse::Tracks(page))) => {
                self.liked.learn(&page.items);
                self.tracks.loaded(offset, page, PageFields::TRACKS)
            }
            (AlbumRequest::Tracks(offset), Err(error)) => self.tracks.failed(offset, error),
            (AlbumRequest::Saved, result) => {
                self.saved = matches!(result, Ok(AlbumResponse::Saved(true)));
                Update::changed(PageFields::SAVED)
            }
            _ => Update::default(),
        }
    }

    fn apply(&mut self, event: &Event) -> Update<AlbumRequest> {
        match event {
            Event::Saved(SavedChanged {
                item: ItemId::Album(id),
                saved,
            }) if *id == self.id => {
                self.saved = *saved;
                Update::changed(PageFields::SAVED)
            }
            _ => Update::default(),
        }
    }
}
