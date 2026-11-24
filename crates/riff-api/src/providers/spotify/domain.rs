use std::sync::{Arc, RwLock};
use std::time::Instant;

use spotify_api::apis::{self, configuration::Configuration};
use spotify_api::models as sp;

use async_trait::async_trait;
use futures::future::try_join_all;

use super::converter::*;
use super::internal_api::{connect_state, SpotifyInternalApi};
use crate::defaults::{self, EntityKind};
use crate::error::DomainError;
use crate::models::*;
use crate::providers::{content_rating_from_explicit, MusicProvider};
use crate::remote::{
    ProviderCapabilities, RemoteCluster, RemoteCommand, RemoteEndpoint, RemotePush,
    TransferPlayback,
};
use crate::token::TokenProvider;

const MAX_LIBRARY_CHECK_ITEMS: usize = 40;

const MAX_SAVED_TRACKS_ITEMS: usize = 50;

const MAX_PLAYLIST_WRITE_ITEMS: usize = 100;

const USER_MARKET: &str = "from_token";

const PLAYLIST_FIELDS: &str =
    "id,name,description,public,snapshot_id,owner(id,display_name),images,\
    tracks(href,limit,next,offset,previous,total,items(is_local))";

pub struct UserProfileCheck {
    pub user_id: String,
    pub is_premium: bool,
    pub explicit_filter_enabled: bool,
    pub explicit_filter_locked: bool,
}

pub async fn check_user_profile(token: &str) -> Result<UserProfileCheck, DomainError> {
    let mut config = Configuration::new();
    config.client = crate::http::api_client();
    config.oauth_access_token = Some(token.to_string());
    let user = apis::users_api::get_current_users_profile(&config).await?;

    let is_premium = !matches!(user.product.as_deref(), Some(p) if p != "premium");

    let (explicit_filter_enabled, explicit_filter_locked) = user
        .explicit_content
        .as_ref()
        .map(|e| {
            (
                e.filter_enabled.unwrap_or(false),
                e.filter_locked.unwrap_or(false),
            )
        })
        .unwrap_or((false, false));

    Ok(UserProfileCheck {
        user_id: user.id.unwrap_or_default(),
        is_premium,
        explicit_filter_enabled,
        explicit_filter_locked,
    })
}

impl<T: std::fmt::Debug> From<apis::Error<T>> for DomainError {
    fn from(e: apis::Error<T>) -> Self {
        match e {
            apis::Error::ResponseError(ref resp) => {
                let status = resp.status.as_u16();
                let message = resp.content.clone();
                match status {
                    401 => DomainError::AuthExpired,
                    429 => DomainError::RateLimited {
                        retry_after_ms: None,
                    },
                    404 => DomainError::NotFound { resource: message },
                    400..=499 => DomainError::ClientError { status, message },
                    500..=599 => DomainError::ServerError { status, message },
                    _ => DomainError::ClientError { status, message },
                }
            }
            apis::Error::Reqwest(e) => DomainError::Network(e.to_string()),
            apis::Error::ReqwestMiddleware(e) => DomainError::Network(e.to_string()),
            apis::Error::Serde(e) => {
                error!("spotify: response body did not match the expected schema: {e}");
                DomainError::Parse(e.to_string())
            }
            apis::Error::Io(e) => {
                error!("spotify: failed to read the response body: {e}");
                DomainError::Network(e.to_string())
            }
        }
    }
}

async fn get_tracks_nullable(
    config: &Configuration,
    ids: &[String],
) -> Result<Vec<Option<sp::TrackObject>>, apis::Error<()>> {
    let mut request = config
        .client
        .get(format!("{}/tracks", config.base_path))
        .query(&[("ids", ids.join(",").as_str()), ("market", USER_MARKET)]);
    if let Some(user_agent) = &config.user_agent {
        request = request.header(reqwest::header::USER_AGENT, user_agent);
    }
    if let Some(token) = &config.oauth_access_token {
        request = request.bearer_auth(token);
    }
    let response = config.client.execute(request.build()?).await?;
    let status = response.status();
    let content = response.text().await?;
    if status.is_client_error() || status.is_server_error() {
        return Err(apis::Error::ResponseError(apis::ResponseContent {
            status,
            content,
            entity: None,
        }));
    }
    Ok(parse_tracks_nullable(&content)?)
}

fn parse_tracks_nullable(content: &str) -> serde_json::Result<Vec<Option<sp::TrackObject>>> {
    #[derive(serde::Deserialize)]
    struct SeveralTracks {
        tracks: Vec<Option<sp::TrackObject>>,
    }
    Ok(serde_json::from_str::<SeveralTracks>(content)?.tracks)
}

struct CachedConfig {
    config: Configuration,
    created_at: Instant,
}

pub struct SpotifyDomain {
    token_provider: Arc<dyn TokenProvider>,
    cached_config: RwLock<Option<CachedConfig>>,
    internal_api: SpotifyInternalApi,
}

use riff_config::api::CONFIG_CACHE_TTL;

impl SpotifyDomain {
    pub fn new(
        token_provider: Arc<dyn TokenProvider>,
        session_token_provider: Arc<dyn TokenProvider>,
    ) -> Self {
        Self {
            token_provider,
            cached_config: RwLock::new(None),
            internal_api: SpotifyInternalApi::new(session_token_provider),
        }
    }

    fn config_or_err(&self) -> Result<Configuration, DomainError> {
        #[cfg(debug_assertions)]
        if let Some(err) = crate::dev::simulated_failure() {
            return Err(err);
        }

        {
            let guard = self.cached_config.read().unwrap();
            if let Some(ref cached) = *guard {
                if cached.created_at.elapsed() < CONFIG_CACHE_TTL {
                    return Ok(cached.config.clone());
                }
            }
        }

        let access_token = self
            .token_provider
            .access_token()
            .ok_or(DomainError::NoToken)?;

        let mut config = Configuration::new();
        config.client = crate::http::api_client();
        config.oauth_access_token = Some(access_token);

        debug!("spotify: rebuilt API config with a refreshed access token");

        let mut guard = self.cached_config.write().unwrap();
        *guard = Some(CachedConfig {
            config: config.clone(),
            created_at: Instant::now(),
        });

        Ok(config)
    }
}

#[async_trait]
impl MusicProvider for SpotifyDomain {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            remote_devices: true,
        }
    }

    fn uris(&self) -> crate::remote::ContentUris {
        crate::remote::ContentUris::new(
            |context| Some(super::context_uri(context)),
            |track_id| Some(format!("spotify:track:{track_id}")),
        )
    }

    fn invalidate_config(&self) {
        debug!("spotify: dropping the cached API config after an auth failure");
        let mut guard = self.cached_config.write().unwrap();
        *guard = None;
    }

    async fn get_album(&self, id: &str) -> Result<Album, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::albums_api::get_an_album(&config, id, None).await?;
        Ok(album_from_object(&raw))
    }

    async fn get_album_tracks(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::albums_api::get_an_albums_tracks(
            &config,
            id,
            None,
            Some(limit as i32),
            Some(offset as i32),
        )
        .await?;
        let album_ref = AlbumRef {
            rri: rid(id.to_string()),
            name: String::new(),
        };
        let items = page
            .items
            .iter()
            .map(|t| Track {
                rri: ResourceId {
                    provider: Provider::Spotify,
                    id: t.id.clone().unwrap_or_default(),
                    uri: t.uri.clone(),
                },
                title: text_or(t.name.clone().unwrap_or_default(), defaults::track_title),
                artists: ensure_artists(
                    t.artists
                        .as_ref()
                        .map(|a| a.iter().map(to_artist_ref).collect())
                        .unwrap_or_default(),
                ),
                album: Some(album_ref.clone()),
                duration_ms: t.duration_ms.unwrap_or(0) as u32,
                track_number: t.track_number.map(|n| n as u32),
                disc_number: t.disc_number.map(|n| n as u32),
                content_rating: content_rating_from_explicit(t.explicit),
                isrc: None,
                art: defaults::image_set_for(EntityKind::Track),
                playable: t.is_playable.unwrap_or(true),
                popularity: None,
                saved: None,
                preview_url: None,
                url: None,
            })
            .collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn get_saved_albums(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::library_api::get_users_saved_albums(
            &config,
            Some(limit as i32),
            Some(offset as i32),
            Some(USER_MARKET),
        )
        .await?;
        let items = page.items.iter().filter_map(album_from_saved).collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn check_saved_albums(&self, ids: &str) -> Result<Vec<bool>, DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::check_users_saved_albums(&config, ids).await?)
    }

    async fn save_albums(&self, ids: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::save_albums_user(&config, ids, None).await?)
    }

    async fn remove_albums(&self, ids: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::remove_albums_user(&config, ids, None).await?)
    }

    async fn get_track(&self, id: &str) -> Result<Track, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::tracks_api::get_track(&config, id, None).await?;
        track_from_object(&raw).ok_or_else(|| DomainError::Parse("track missing id".into()))
    }

    async fn get_tracks(&self, ids: &[String]) -> Result<Vec<(String, Track)>, DomainError> {
        let config = self.config_or_err()?;
        let tracks = get_tracks_nullable(&config, ids).await?;
        Ok(ids
            .iter()
            .cloned()
            .zip(
                tracks
                    .iter()
                    .map(|track| track.as_ref().and_then(track_from_object)),
            )
            .filter_map(|(id, track)| Some((id, track?)))
            .collect())
    }

    async fn get_saved_tracks(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::library_api::get_users_saved_tracks(
            &config,
            None,
            Some(limit as i32),
            Some(offset as i32),
        )
        .await?;
        let items = page
            .items
            .iter()
            .filter_map(|st| st.track.as_ref().and_then(|t| track_from_object(t)))
            .collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn check_saved_tracks(&self, ids: Vec<String>) -> Result<Vec<bool>, DomainError> {
        let config = self.config_or_err()?;
        let config = &config;
        let pages = try_join_all(ids.chunks(MAX_LIBRARY_CHECK_ITEMS).map(|chunk| async move {
            let uris = chunk
                .iter()
                .map(|id| format!("spotify:track:{id}"))
                .collect::<Vec<_>>()
                .join(",");
            let page = apis::library_api::check_library_contains(config, &uris).await?;
            if page.len() != chunk.len() {
                return Err(DomainError::Parse(format!(
                    "library check returned {} results for {} tracks",
                    page.len(),
                    chunk.len()
                )));
            }
            Ok(page)
        }))
        .await?;
        Ok(pages.into_iter().flatten().collect())
    }

    async fn save_tracks(&self, ids: Vec<String>) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        for ids in ids.chunks(MAX_SAVED_TRACKS_ITEMS) {
            let body = sp::SaveTracksUserRequest {
                ids: Some(ids.to_vec()),
                timestamped_ids: None,
            };
            apis::library_api::save_tracks_user(&config, Some(body)).await?;
        }
        Ok(())
    }

    async fn remove_tracks(&self, ids: Vec<String>) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        for ids in ids.chunks(MAX_SAVED_TRACKS_ITEMS) {
            apis::library_api::remove_tracks_user(&config, &ids.join(","), None).await?;
        }
        Ok(())
    }

    async fn get_saved_playlists(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Playlist>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::playlists_api::get_a_list_of_current_users_playlists(
            &config,
            Some(limit as i32),
            Some(offset as i32),
        )
        .await?;
        let items = page.items.iter().map(playlist_from_simplified).collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::playlists_api::get_playlist(
            &config,
            id,
            Some(USER_MARKET),
            Some(PLAYLIST_FIELDS),
            None,
        )
        .await?;
        Ok(playlist_from_object(&raw))
    }

    async fn get_playlist_tracks(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::playlists_api::get_playlists_tracks(
            &config,
            id,
            None,
            None,
            Some(limit as i32),
            Some(offset as i32),
            None,
        )
        .await?;
        let items = page.items.iter().map(track_from_playlist_item).collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn get_playlist_item_uris(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<Option<String>>, usize), DomainError> {
        let config = self.config_or_err()?;
        let page = apis::playlists_api::get_playlists_tracks(
            &config,
            id,
            None,
            None,
            Some(limit as i32),
            Some(offset as i32),
            None,
        )
        .await?;
        let uris = page
            .items
            .iter()
            .map(|item| {
                if item.is_local == Some(true) {
                    return None;
                }
                match item.track.as_deref()? {
                    sp::PlaylistTrackObjectTrack::Track(track) => track.uri.clone(),
                    sp::PlaylistTrackObjectTrack::Episode(episode) => Some(episode.uri.clone()),
                }
            })
            .collect();
        Ok((uris, page.total as usize))
    }

    async fn add_to_playlist(
        &self,
        id: &str,
        uris: Vec<String>,
        position: Option<i32>,
    ) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        for (i, uris) in uris.chunks(MAX_PLAYLIST_WRITE_ITEMS).enumerate() {
            let body = sp::AddTracksToPlaylistRequest {
                uris: Some(uris.to_vec()),
                position: position.map(|p| p + (i * MAX_PLAYLIST_WRITE_ITEMS) as i32),
            };
            apis::playlists_api::add_tracks_to_playlist(&config, id, None, None, Some(body))
                .await?;
        }
        Ok(())
    }

    async fn remove_from_playlist(
        &self,
        id: &str,
        uris: Vec<String>,
        snapshot_id: Option<&str>,
    ) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        for (i, uris) in uris.chunks(MAX_PLAYLIST_WRITE_ITEMS).enumerate() {
            let tracks = uris
                .iter()
                .map(|uri| sp::RemoveTracksPlaylistRequestTracksInner {
                    uri: Some(uri.clone()),
                })
                .collect();
            let body = sp::RemoveTracksPlaylistRequest {
                tracks,
                snapshot_id: snapshot_id.filter(|_| i == 0).map(|s| s.to_string()),
            };
            apis::playlists_api::remove_tracks_playlist(&config, id, Some(body)).await?;
        }
        Ok(())
    }

    async fn create_playlist(
        &self,
        user_id: &str,
        name: &str,
        public: Option<bool>,
        collaborative: Option<bool>,
        description: Option<&str>,
    ) -> Result<Playlist, DomainError> {
        let config = self.config_or_err()?;
        let body = sp::CreatePlaylistRequest {
            name: name.to_string(),
            public,
            collaborative,
            description: description.map(|s| s.to_string()),
        };
        let raw =
            apis::playlists_api::create_playlist_for_user(&config, user_id, Some(body)).await?;
        Ok(playlist_from_object(&raw))
    }

    async fn upload_playlist_cover(
        &self,
        id: &str,
        jpeg_base64: String,
    ) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::playlists_api::upload_custom_playlist_cover(
            &config,
            id,
            "image/jpeg",
            jpeg_base64,
        )
        .await?)
    }

    async fn follow_playlist(&self, id: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::playlists_api::follow_playlist(&config, id, None).await?)
    }

    async fn unfollow_playlist(&self, id: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::playlists_api::unfollow_playlist(&config, id).await?)
    }

    async fn update_playlist_details(
        &self,
        id: &str,
        name: Option<&str>,
        public: Option<bool>,
        collaborative: Option<bool>,
        description: Option<&str>,
    ) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        let body = sp::ChangePlaylistDetailsRequest {
            name: name.map(|s| s.to_string()),
            public,
            collaborative,
            description: description.map(|s| s.to_string()),
        };
        Ok(apis::playlists_api::change_playlist_details(&config, id, Some(body)).await?)
    }

    async fn set_playlist_public(&self, id: &str, public: bool) -> Result<(), DomainError> {
        let now_public = self.internal_api.set_playlist_public(id, public).await?;
        if now_public != public {
            return Err(DomainError::Conflict(format!(
                "playlist {id}: asked for public = {public}, got {now_public}"
            )));
        }
        Ok(())
    }

    async fn get_artist(&self, id: &str) -> Result<Artist, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::artists_api::get_an_artist(&config, id).await?;
        Ok(artist_from_object(&raw))
    }

    async fn get_artist_albums(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::artists_api::get_an_artists_albums(
            &config,
            id,
            Some("album,single,compilation"),
            Some(USER_MARKET),
            Some(limit as i32),
            Some(offset as i32),
        )
        .await?;
        let items = page.items.iter().map(album_from_discography).collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn get_artist_top_tracks(&self, id: &str) -> Result<Vec<Track>, DomainError> {
        let config = self.config_or_err()?;
        let resp = apis::artists_api::get_an_artists_top_tracks(&config, id, None).await?;
        Ok(resp.tracks.iter().filter_map(track_from_object).collect())
    }

    async fn get_followed_artists(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Artist>, Option<String>), DomainError> {
        let config = self.config_or_err()?;
        let resp =
            apis::library_api::get_followed(&config, "artist", after, Some(limit as i32)).await?;
        let artists = resp
            .artists
            .items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(artist_from_object)
            .collect();
        let next = resp.artists.cursors.as_ref().and_then(|c| c.after.clone());
        Ok((artists, next))
    }

    async fn follow_artists(&self, ids: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::follow_artists_users(&config, "artist", ids, None).await?)
    }

    async fn unfollow_artists(&self, ids: &str) -> Result<(), DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::unfollow_artists_users(&config, "artist", ids, None).await?)
    }

    async fn check_following_artists(&self, ids: &str) -> Result<Vec<bool>, DomainError> {
        let config = self.config_or_err()?;
        Ok(apis::library_api::check_current_user_follows(&config, "artist", ids).await?)
    }

    async fn get_current_user(&self) -> Result<User, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::users_api::get_current_users_profile(&config).await?;
        Ok(current_user_from_private(&raw))
    }

    async fn get_user(&self, id: &str) -> Result<User, DomainError> {
        let config = self.config_or_err()?;
        let raw = apis::users_api::get_users_profile(&config, id).await?;
        Ok(user_from_object(&raw))
    }

    async fn get_user_playlists(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Playlist>, DomainError> {
        let config = self.config_or_err()?;
        let page = apis::playlists_api::get_list_users_playlists(
            &config,
            id,
            Some(limit as i32),
            Some(offset as i32),
        )
        .await?;
        let items = page.items.iter().map(playlist_from_simplified).collect();
        Ok(Page::offset_paged(
            items,
            page.offset as usize,
            page.total as usize,
        ))
    }

    async fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResults, DomainError> {
        let config = self.config_or_err()?;
        let types = vec![
            "album".to_string(),
            "artist".to_string(),
            "track".to_string(),
            "playlist".to_string(),
        ];
        let resp = apis::search_api::search(
            &config,
            query,
            types,
            None,
            Some(limit as i32),
            Some(offset as i32),
            None,
        )
        .await?;
        Ok(search_results_from_response(&resp))
    }

    async fn search_scoped(
        &self,
        query: &str,
        kind: SearchType,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResults, DomainError> {
        let config = self.config_or_err()?;
        let types = vec![search_type_str(kind).to_string()];
        let market = matches!(kind, SearchType::Album).then_some(USER_MARKET);
        let resp = apis::search_api::search(
            &config,
            query,
            types,
            market,
            Some(limit as i32),
            Some(offset as i32),
            None,
        )
        .await?;
        Ok(search_results_from_response(&resp))
    }

    async fn remote_endpoint(&self) -> Result<RemoteEndpoint, DomainError> {
        connect_state::endpoint(&self.internal_api).await
    }

    fn parse_remote_push(&self, text: &str) -> RemotePush {
        connect_state::parse_push(text)
    }

    fn remote_keepalive(&self) -> &'static str {
        connect_state::PING
    }

    async fn remote_register(
        &self,
        observer_id: &str,
        connection_id: &str,
    ) -> Result<RemoteCluster, DomainError> {
        connect_state::register_observer(&self.internal_api, observer_id, connection_id, "Riff")
            .await
    }

    async fn remote_unregister(
        &self,
        observer_id: &str,
        connection_id: &str,
    ) -> Result<(), DomainError> {
        connect_state::unregister_observer(&self.internal_api, observer_id, connection_id).await
    }

    async fn remote_command(
        &self,
        from: &str,
        to: &str,
        connection_id: &str,
        command: RemoteCommand,
    ) -> Result<(), DomainError> {
        connect_state::command(&self.internal_api, from, to, connection_id, command).await
    }

    async fn remote_transfer(
        &self,
        from: &str,
        to: &str,
        connection_id: &str,
        playback: TransferPlayback,
    ) -> Result<(), DomainError> {
        connect_state::transfer(&self.internal_api, from, to, connection_id, playback).await
    }

    async fn remote_volume(
        &self,
        from: &str,
        to: &str,
        connection_id: &str,
        percent: u8,
    ) -> Result<(), DomainError> {
        connect_state::set_volume(&self.internal_api, from, to, connection_id, percent).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_classified() {
        assert!(DomainError::Network("x".into()).is_transient());
        assert!(DomainError::RateLimited {
            retry_after_ms: None
        }
        .is_transient());
        assert!(DomainError::ServerError {
            status: 500,
            message: "x".into()
        }
        .is_transient());
        assert!(!DomainError::AuthExpired.is_transient());
        assert!(!DomainError::NotFound {
            resource: "x".into()
        }
        .is_transient());
    }

    #[test]
    fn tracks_that_dont_resolve_are_left_out() {
        let tracks = parse_tracks_nullable(r#"{"tracks":[{"id":"a","name":"A"},null]}"#).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].as_ref().and_then(|t| t.id.as_deref()), Some("a"));
        assert!(tracks[1].is_none());
    }
}
