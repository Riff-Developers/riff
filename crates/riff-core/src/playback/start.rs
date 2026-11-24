use crate::play_queue::SongsSource;
use crate::remote::Intent;

use super::PlaybackAction;

#[derive(Clone, Debug, PartialEq)]
pub struct PlayRequest {
    pub source: Option<SongsSource>,
    pub tracks: Vec<String>,
    pub start: Option<String>,
    pub shuffle: Option<bool>,
}

impl PlayRequest {
    pub fn intent(self, user_id: Option<&str>) -> Intent {
        let context = self
            .source
            .and_then(|source| source.remote_context(user_id));
        let track_ids = if context.is_some() {
            vec![]
        } else {
            self.tracks
        };
        Intent::Play {
            context,
            track_ids,
            track_id: self.start,
            index: None,
            position_ms: 0,
            shuffle: self.shuffle,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayContext {
    pub source: SongsSource,
    pub name: Option<String>,
    pub tracks: Vec<String>,
}

impl PlayContext {
    pub fn whole(source: SongsSource, name: Option<String>) -> Self {
        Self {
            source,
            name,
            tracks: vec![],
        }
    }

    pub fn play(self, start: Option<String>, shuffle: Option<bool>) -> Vec<PlaybackAction> {
        let name = self
            .name
            .map(|name| PlaybackAction::SetContextName(self.source.clone(), name));
        let play = PlaybackAction::PlayFrom(PlayRequest {
            source: Some(self.source),
            tracks: self.tracks,
            start,
            shuffle,
        });
        name.into_iter().chain([play]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riff_api::remote::RemoteContext;

    #[test]
    fn contexts_play_whole_and_lists_by_track() {
        let album = PlayContext::whole(SongsSource::Album("a".into()), Some("A".into()));
        let actions = album.play(Some("t".into()), Some(true));
        assert!(matches!(
            actions[..],
            [
                PlaybackAction::SetContextName(..),
                PlaybackAction::PlayFrom(_)
            ]
        ));
        let PlaybackAction::PlayFrom(request) = actions[1].clone() else {
            unreachable!()
        };
        assert_eq!(
            request.intent(None),
            Intent::Play {
                context: Some(RemoteContext::Album("a".into())),
                track_ids: vec![],
                track_id: Some("t".into()),
                index: None,
                position_ms: 0,
                shuffle: Some(true),
            }
        );

        let search = PlayRequest {
            source: Some(SongsSource::Search("q".into())),
            tracks: vec!["1".into(), "2".into()],
            start: None,
            shuffle: None,
        };
        let Intent::Play {
            context, track_ids, ..
        } = search.intent(Some("me"))
        else {
            unreachable!()
        };
        assert_eq!((context, track_ids.len()), (None, 2));

        let saved = PlayRequest {
            source: Some(SongsSource::SavedTracks),
            tracks: vec!["1".into()],
            start: None,
            shuffle: None,
        };
        assert!(matches!(
            saved.intent(Some("me")),
            Intent::Play {
                context: Some(RemoteContext::SavedTracks { .. }),
                ..
            }
        ));
    }
}
