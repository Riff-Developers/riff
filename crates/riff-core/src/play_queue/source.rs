use riff_api::remote::RemoteContext;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SongsSource {
    Playlist(String),
    Album(String),
    Artist(String),
    SavedTracks,
    Search(String),
}

impl SongsSource {
    pub fn remote_context(&self, user_id: Option<&str>) -> Option<RemoteContext> {
        Some(match self {
            Self::Playlist(id) => RemoteContext::Playlist(id.clone()),
            Self::Album(id) => RemoteContext::Album(id.clone()),
            Self::Artist(id) => RemoteContext::Artist(id.clone()),
            Self::SavedTracks => RemoteContext::SavedTracks {
                user_id: user_id?.to_string(),
            },
            Self::Search(_) => return None,
        })
    }

    pub fn from_remote(context: &RemoteContext) -> Self {
        match context {
            RemoteContext::Playlist(id) => Self::Playlist(id.clone()),
            RemoteContext::Album(id) => Self::Album(id.clone()),
            RemoteContext::Artist(id) => Self::Artist(id.clone()),
            RemoteContext::SavedTracks { .. } => Self::SavedTracks,
        }
    }

    pub fn spotify_url(&self) -> Option<String> {
        let (kind, id) = match self {
            Self::Playlist(id) => ("playlist", id),
            Self::Album(id) => ("album", id),
            Self::Artist(id) => ("artist", id),
            Self::SavedTracks | Self::Search(_) => return None,
        };
        Some(format!("https://open.spotify.com/{}/{}", kind, id))
    }

    pub fn is_paginated(&self) -> bool {
        matches!(self, Self::Playlist(_) | Self::Album(_) | Self::SavedTracks)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageRequest {
    pub offset: usize,
    pub batch_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_contexts_round_trip() {
        for source in [
            SongsSource::Playlist("p".into()),
            SongsSource::Album("a".into()),
            SongsSource::Artist("r".into()),
            SongsSource::SavedTracks,
        ] {
            let context = source.remote_context(Some("me")).unwrap();
            assert_eq!(SongsSource::from_remote(&context), source);
        }
        assert_eq!(SongsSource::SavedTracks.remote_context(None), None);
        assert_eq!(
            SongsSource::Search("q".into()).remote_context(Some("me")),
            None
        );
    }
}
