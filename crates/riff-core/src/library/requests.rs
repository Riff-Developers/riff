use std::collections::HashSet;

use riff_api::models::{ImageSet, Playlist, Track};
use riff_api::{ApiService, DomainError, Load};

use super::PlaylistSummary;
use crate::browse::LibraryAction;
use crate::events::{ItemId, PlaylistFields};
use crate::pagination::{load_all, PAGE_SIZE};

pub async fn load_saved_playlists(
    api: &ApiService,
    load: Load,
) -> Result<Vec<Playlist>, DomainError> {
    load_all(|offset| api.get_saved_playlists(offset, PAGE_SIZE, load)).await
}

pub struct TracksAdded {
    pub tracks: Vec<Track>,
    pub skipped: usize,
}

pub async fn add_tracks_to_playlist(
    api: &ApiService,
    id: &str,
    mut tracks: Vec<Track>,
    load: Load,
) -> Result<TracksAdded, DomainError> {
    tracks.retain(|track| track.rri.uri.is_some());
    let asked = tracks.len();
    let uris: Vec<String> = tracks.iter().filter_map(|t| t.rri.uri.clone()).collect();
    let mut added: HashSet<String> = api
        .add_to_playlist(id, uris, load)
        .await?
        .into_iter()
        .collect();
    tracks.retain(|track| track.rri.uri.as_ref().is_some_and(|uri| added.remove(uri)));
    Ok(TracksAdded {
        skipped: asked - tracks.len(),
        tracks,
    })
}

pub async fn refreshed_playlist_cover(api: &ApiService, id: &str, poll: bool) -> Option<ImageSet> {
    api.refreshed_playlist_cover(id, poll, Load::background())
        .await
}

pub async fn set_saved(
    api: &ApiService,
    item: ItemId,
    saved: bool,
) -> Result<LibraryAction, DomainError> {
    match (&item, saved) {
        (ItemId::Album(id), true) => api.save_albums(id).await?,
        (ItemId::Album(id), false) => api.remove_albums(id).await?,
        (ItemId::Artist(id), true) => api.follow_artists(id).await?,
        (ItemId::Artist(id), false) => api.unfollow_artists(id).await?,
        (ItemId::Tracks(ids), true) => api.save_tracks(ids.clone()).await?,
        (ItemId::Tracks(ids), false) => api.remove_tracks(ids.clone()).await?,
        (ItemId::Playlist(id), true) => api.follow_playlist(id).await?,
        (ItemId::Playlist(id), false) => api.unfollow_playlist(id).await?,
    }
    Ok(LibraryAction::SetSaved(item, saved))
}

pub async fn follow_playlist(
    api: &ApiService,
    playlist: Playlist,
) -> Result<LibraryAction, DomainError> {
    api.follow_playlist(&playlist.rri.id).await?;
    Ok(LibraryAction::SavePlaylist(Box::new(playlist)))
}

pub async fn create_playlist(
    api: &ApiService,
    user: &str,
    details: &PlaylistDetails,
) -> Result<Playlist, DomainError> {
    api.create_playlist(
        user,
        &details.name,
        details.description.as_deref(),
        details.public.unwrap_or(false),
    )
    .await
}

pub async fn remove_from_playlist(
    api: &ApiService,
    id: String,
    uris: Vec<String>,
) -> Result<LibraryAction, DomainError> {
    api.remove_from_playlist(&id, uris).await?;
    Ok(LibraryAction::PlaylistEdited(
        id,
        PlaylistFields::INFO | PlaylistFields::TRACKS,
    ))
}

pub async fn rename_playlist(
    api: &ApiService,
    id: String,
    title: String,
) -> Result<LibraryAction, DomainError> {
    api.update_playlist_details(&id, &title, None, None).await?;
    Ok(LibraryAction::RenamePlaylist(PlaylistSummary { id, title }))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaylistDetails {
    pub name: String,
    pub description: Option<String>,
    pub public: Option<bool>,
}

impl PlaylistDetails {
    pub fn keeps_description(&self, current: Option<&str>) -> bool {
        self.description.is_none() && current.is_some()
    }
}

pub async fn edit_playlist(
    api: &ApiService,
    id: String,
    details: PlaylistDetails,
) -> Result<Vec<LibraryAction>, DomainError> {
    api.update_playlist_details(
        &id,
        &details.name,
        details.description.as_deref(),
        details.public,
    )
    .await?;
    Ok(vec![
        LibraryAction::RenamePlaylist(PlaylistSummary {
            id: id.clone(),
            title: details.name,
        }),
        LibraryAction::PlaylistEdited(id, PlaylistFields::INFO),
    ])
}

pub async fn upload_playlist_cover(
    api: &ApiService,
    id: String,
    cover: Vec<u8>,
    load: Load,
) -> Result<Option<LibraryAction>, DomainError> {
    let art = api.upload_playlist_cover(&id, cover, load).await?;
    Ok(art.map(|art| LibraryAction::SetPlaylistArt(id, art)))
}
