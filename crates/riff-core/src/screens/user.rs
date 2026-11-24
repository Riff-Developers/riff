use std::sync::Arc;

use riff_api::models::{Page, Playlist, User};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, PageFields, Pages, Screen, Update};
use crate::navigation::Route;
use crate::pagination::PAGE_SIZE;

pub struct UserScreen {
    id: String,
    pub user: Option<User>,
    pub playlists: Pages<Playlist>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserRequest {
    Info,
    Playlists(usize),
}

pub enum UserResponse {
    Info(User),
    Playlists(Page<Playlist>),
}

impl UserScreen {
    pub fn new(id: String) -> Self {
        Self {
            id,
            user: None,
            playlists: Pages::default(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Screen for UserScreen {
    type Request = UserRequest;
    type Response = UserResponse;

    fn route(&self) -> Route {
        Route::User(self.id.clone())
    }

    fn open(&mut self) -> Vec<UserRequest> {
        vec![
            UserRequest::Info,
            UserRequest::Playlists(self.playlists.restart()),
        ]
    }

    fn load_more(&mut self) -> Vec<UserRequest> {
        self.playlists.next_request(UserRequest::Playlists)
    }

    fn fetch(&self, api: Arc<ApiService>, request: UserRequest, load: Load) -> Fetch<UserResponse> {
        let id = self.id.clone();
        Box::pin(async move {
            Ok(match request {
                UserRequest::Info => UserResponse::Info(api.get_user(&id, load).await?),
                UserRequest::Playlists(offset) => UserResponse::Playlists(
                    api.get_user_playlists(&id, offset, PAGE_SIZE, load).await?,
                ),
            })
        })
    }

    fn receive(
        &mut self,
        request: UserRequest,
        result: Result<UserResponse, DomainError>,
    ) -> Update<UserRequest> {
        match (request, result) {
            (UserRequest::Info, Ok(UserResponse::Info(user))) => {
                let title = user.display_name.clone();
                self.user = Some(user);
                Update::changed(PageFields::INFO).titled(title)
            }
            (UserRequest::Playlists(offset), Ok(UserResponse::Playlists(page))) => {
                self.playlists.loaded(offset, page, PageFields::CARDS)
            }
            (UserRequest::Playlists(offset), Err(error)) => self.playlists.failed(offset, error),
            (_, Err(error)) => Update::error(error),
            _ => Update::default(),
        }
    }
}
