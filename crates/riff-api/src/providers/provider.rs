use async_trait::async_trait;

use crate::error::DomainError;
use crate::models::*;
use crate::remote::{
    ProviderCapabilities, RemoteCluster, RemoteCommand, RemoteEndpoint, RemotePush,
    TransferPlayback,
};

#[async_trait]
pub trait MusicProvider: Send + Sync + 'static {
    async fn get_album(&self, id: &str) -> Result<Album, DomainError>;
    async fn get_album_tracks(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError>;
    async fn get_saved_albums(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, DomainError>;
    async fn check_saved_albums(&self, ids: &str) -> Result<Vec<bool>, DomainError>;
    async fn save_albums(&self, ids: &str) -> Result<(), DomainError>;
    async fn remove_albums(&self, ids: &str) -> Result<(), DomainError>;

    async fn get_track(&self, id: &str) -> Result<Track, DomainError>;
    async fn get_saved_tracks(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError>;
    async fn check_saved_tracks(&self, ids: Vec<String>) -> Result<Vec<bool>, DomainError>;
    async fn save_tracks(&self, ids: Vec<String>) -> Result<(), DomainError>;
    async fn remove_tracks(&self, ids: Vec<String>) -> Result<(), DomainError>;

    async fn get_saved_playlists(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Playlist>, DomainError>;
    async fn get_playlist(&self, id: &str) -> Result<Playlist, DomainError>;

    async fn get_playlist_item_uris(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<Option<String>>, usize), DomainError>;
    async fn get_playlist_tracks(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError>;
    async fn add_to_playlist(
        &self,
        id: &str,
        uris: Vec<String>,
        position: Option<i32>,
    ) -> Result<(), DomainError>;
    async fn remove_from_playlist(
        &self,
        id: &str,
        uris: Vec<String>,
        snapshot_id: Option<&str>,
    ) -> Result<(), DomainError>;
    async fn create_playlist(
        &self,
        user_id: &str,
        name: &str,
        public: Option<bool>,
        collaborative: Option<bool>,
        description: Option<&str>,
    ) -> Result<Playlist, DomainError>;
    async fn upload_playlist_cover(&self, id: &str, jpeg_base64: String)
        -> Result<(), DomainError>;
    async fn follow_playlist(&self, id: &str) -> Result<(), DomainError>;
    async fn unfollow_playlist(&self, id: &str) -> Result<(), DomainError>;
    async fn update_playlist_details(
        &self,
        id: &str,
        name: Option<&str>,
        public: Option<bool>,
        collaborative: Option<bool>,
        description: Option<&str>,
    ) -> Result<(), DomainError>;

    async fn set_playlist_public(&self, id: &str, public: bool) -> Result<(), DomainError>;

    async fn get_artist(&self, id: &str) -> Result<Artist, DomainError>;
    async fn get_artist_albums(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, DomainError>;
    async fn get_artist_top_tracks(&self, id: &str) -> Result<Vec<Track>, DomainError>;
    async fn get_followed_artists(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Artist>, Option<String>), DomainError>;
    async fn follow_artists(&self, ids: &str) -> Result<(), DomainError>;
    async fn unfollow_artists(&self, ids: &str) -> Result<(), DomainError>;
    async fn check_following_artists(&self, ids: &str) -> Result<Vec<bool>, DomainError>;

    async fn get_current_user(&self) -> Result<User, DomainError>;
    async fn get_user(&self, id: &str) -> Result<User, DomainError>;
    async fn get_user_playlists(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Playlist>, DomainError>;

    async fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResults, DomainError>;
    async fn search_scoped(
        &self,
        query: &str,
        kind: SearchType,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResults, DomainError>;

    /// Tracks by id, at most 50 at a time, each with the id it was asked
    /// for: a track can come back as a regional copy with an id of its own.
    async fn get_tracks(&self, ids: &[String]) -> Result<Vec<(String, Track)>, DomainError>;

    /// What the provider can do beyond its catalog.
    fn capabilities(&self) -> ProviderCapabilities;

    /// How the provider's own players name what they play.
    fn uris(&self) -> crate::remote::ContentUris {
        crate::remote::ContentUris::none()
    }

    async fn remote_endpoint(&self) -> Result<RemoteEndpoint, DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }
    fn parse_remote_push(&self, _text: &str) -> RemotePush {
        RemotePush::Other
    }
    /// The keepalive to send over the live connection.
    fn remote_keepalive(&self) -> &'static str {
        ""
    }
    async fn remote_register(
        &self,
        _observer_id: &str,
        _connection_id: &str,
    ) -> Result<RemoteCluster, DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }
    async fn remote_unregister(
        &self,
        _observer_id: &str,
        _connection_id: &str,
    ) -> Result<(), DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }
    async fn remote_command(
        &self,
        _from: &str,
        _to: &str,
        _connection_id: &str,
        _command: RemoteCommand,
    ) -> Result<(), DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }
    async fn remote_transfer(
        &self,
        _from: &str,
        _to: &str,
        _connection_id: &str,
        _playback: TransferPlayback,
    ) -> Result<(), DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }
    async fn remote_volume(
        &self,
        _from: &str,
        _to: &str,
        _connection_id: &str,
        _percent: u8,
    ) -> Result<(), DomainError> {
        Err(DomainError::Unsupported("remote devices"))
    }

    /// Drop any cached auth configuration (e.g. after a 401).
    fn invalidate_config(&self);
}
