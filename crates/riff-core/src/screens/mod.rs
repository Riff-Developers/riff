mod album;
mod artist;
mod liked;
mod pages;
mod playlist;
mod saved;
mod search;
mod user;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use riff_api::{ApiService, DomainError, Load};

use crate::browse::{BrowserAction, NavigationAction};
use crate::events::Event;
use crate::navigation::Route;

pub use album::{AlbumRequest, AlbumResponse, AlbumScreen};
pub use artist::{ArtistRequest, ArtistResponse, ArtistScreen};
pub use liked::LikedTracks;
pub use pages::{Pages, TrackPages};
pub use playlist::{PlaylistRequest, PlaylistResponse, PlaylistScreen};
pub use saved::{SavedAlbumsScreen, SavedArtistsRequest, SavedArtistsScreen, SavedTracksScreen};
pub use search::{SearchRequest, SearchScreen};
pub use user::{UserRequest, UserResponse, UserScreen};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct PageFields: u8 {
        const INFO = 1 << 0;
        const TRACKS = 1 << 1;
        const CARDS = 1 << 2;
        const SAVED = 1 << 3;
    }
}

pub type Fetch<T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send>>;

pub trait Screen {
    type Request: Clone + Send + 'static;
    type Response: Send + 'static;

    fn route(&self) -> Route;

    fn open(&mut self) -> Vec<Self::Request>;

    fn load_more(&mut self) -> Vec<Self::Request> {
        vec![]
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        request: Self::Request,
        load: Load,
    ) -> Fetch<Self::Response>;

    fn receive(
        &mut self,
        request: Self::Request,
        result: Result<Self::Response, DomainError>,
    ) -> Update<Self::Request>;

    fn apply(&mut self, _event: &Event) -> Update<Self::Request> {
        Update::default()
    }
}

#[derive(Debug)]
pub struct Update<R> {
    pub changed: PageFields,
    pub title: Option<String>,
    pub actions: Vec<BrowserAction>,
    pub requests: Vec<R>,
    pub error: Option<DomainError>,
}

impl<R> Default for Update<R> {
    fn default() -> Self {
        Self {
            changed: PageFields::empty(),
            title: None,
            actions: vec![],
            requests: vec![],
            error: None,
        }
    }
}

impl<R> Update<R> {
    pub fn changed(changed: PageFields) -> Self {
        Self {
            changed,
            ..Default::default()
        }
    }

    pub fn load(requests: Vec<R>) -> Self {
        Self {
            requests,
            ..Default::default()
        }
    }

    pub fn error(error: DomainError) -> Self {
        Self {
            error: Some(error),
            ..Default::default()
        }
    }

    pub fn titled(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn and_load(mut self, requests: Vec<R>) -> Self {
        self.requests.extend(requests);
        self
    }

    pub fn info_failed(route: Route, error: DomainError) -> Self {
        match error {
            DomainError::ClientError { status: 400, .. } | DomainError::NotFound { .. } => Self {
                actions: vec![NavigationAction::Leave(route).into()],
                ..Default::default()
            },
            error => Self::error(error),
        }
    }
}
