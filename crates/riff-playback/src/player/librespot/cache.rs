use librespot::core::authentication::Credentials;
use librespot::core::cache::Cache;
use riff_auth::SESSION_CLIENT_ID;

use crate::player::SpotifyError;

static CACHE_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub fn set_cache_root(root: std::path::PathBuf) {
    if CACHE_ROOT.set(root).is_err() {
        warn!("The player cache root was already set");
    }
}

pub(super) fn librespot_cache_root() -> std::path::PathBuf {
    CACHE_ROOT.get().cloned().unwrap_or_else(|| {
        warn!("The player cache root was not set; using the temporary directory");
        std::env::temp_dir().join("riff").join("librespot")
    })
}

pub fn clear_audio_cache() {
    let audio_dir = librespot_cache_root().join("audio");
    if let Err(e) = std::fs::remove_dir_all(&audio_dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            warn!("Failed to clear librespot audio cache: {e}");
        }
    }
}

pub(super) fn open_librespot_cache() -> Option<Cache> {
    let root = librespot_cache_root();
    Cache::new(
        Some(root.join("credentials")),
        Some(root.join("volume")),
        Some(root.join("audio")),
        Some(riff_config::SettingsStore::new().audio_cache_size_limit_bytes()),
    )
    .map_err(|e| warn!("Couldn't open the player cache: {e}"))
    .ok()
}

fn session_minted_by_path() -> std::path::PathBuf {
    librespot_cache_root().join("credentials").join("minted-by")
}

pub(super) fn note_session_minted() {
    if let Err(e) = std::fs::write(session_minted_by_path(), SESSION_CLIENT_ID) {
        warn!("Cannot note which client id minted the session credentials: {e}");
    }
}

pub(super) fn session_cache_usable() -> bool {
    let Some(cache) = open_librespot_cache() else {
        return false;
    };
    if cache.credentials().is_none() {
        return false;
    }
    minted_the_session_way(
        std::fs::read_to_string(session_minted_by_path())
            .ok()
            .as_deref(),
    )
}

fn minted_the_session_way(noted: Option<&str>) -> bool {
    noted.is_some_and(|noted| noted.trim() == SESSION_CLIENT_ID)
}

pub(super) fn cached_credentials() -> Result<Credentials, SpotifyError> {
    open_librespot_cache()
        .and_then(|cache| cache.credentials())
        .ok_or(SpotifyError::LoggedOut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_from_before_the_split_are_not_reused() {
        assert!(
            !minted_the_session_way(None),
            "a cache with no note predates the split"
        );
        assert!(
            !minted_the_session_way(Some("782ae96ea60f4cdf986a766049607005")),
            "our registered client id cannot mint the session's credentials"
        );
        assert!(
            minted_the_session_way(Some(SESSION_CLIENT_ID)),
            "Spotify's own client id is the one that works"
        );
        assert!(
            minted_the_session_way(Some(&format!("{SESSION_CLIENT_ID}\n"))),
            "a trailing newline is still the same client id"
        );
    }
}
