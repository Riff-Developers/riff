use riff_api::models::{Page, Track};
use riff_api::{ApiService, DomainError, Load};

use super::SongsSource;
use crate::pagination::{load_all, PAGE_SIZE};

async fn source_page(
    api: &ApiService,
    source: &SongsSource,
    offset: usize,
    load: Load,
) -> Result<Page<Track>, DomainError> {
    match source {
        SongsSource::Playlist(id) => api.get_playlist_tracks(id, offset, PAGE_SIZE, load).await,
        SongsSource::Album(id) => api.get_album_tracks(id, offset, PAGE_SIZE, load).await,
        SongsSource::SavedTracks => api.get_saved_tracks(offset, PAGE_SIZE, load).await,
        SongsSource::Artist(_) | SongsSource::Search(_) => {
            unreachable!("{:?} isn't paginated", source)
        }
    }
}

pub async fn playable_tracks(
    api: &ApiService,
    source: &SongsSource,
    load: Load,
) -> Result<Vec<Track>, DomainError> {
    let mut tracks = match source {
        SongsSource::Artist(id) => api.get_artist_top_tracks(id, load).await?,
        source => load_all(|offset| source_page(api, source, offset, load)).await?,
    };
    tracks.retain(|t| t.playable);
    Ok(tracks)
}
