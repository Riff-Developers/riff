use riff_api::models::RepeatMode;
use riff_core::events::{self, PlaybackChanged, PlaybackFields};
use riff_core::playback::PlaybackState;

use crate::types::{LoopStatus, PlaybackStatus, TrackMetadata, NO_TRACK};

#[derive(Debug, Default)]
pub struct MprisUpdate {
    pub track: Option<Option<TrackMetadata>>,
    pub status: Option<PlaybackStatus>,
    pub loop_status: Option<LoopStatus>,
    pub shuffled: Option<bool>,
    pub position: Option<Position>,
    pub volume: Option<f64>,
    pub has_prev: bool,
    pub has_next: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub micros: u128,
    pub by_user: bool,
}

fn track_meta(playback: &PlaybackState) -> Option<TrackMetadata> {
    let song = playback.current_song()?;
    let id = &song.rri.id;
    Some(TrackMetadata {
        id: track_path(id),
        length: 1000 * song.duration_ms as u64,
        title: song.title.clone(),
        album: song
            .album
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        artist: song.artists.iter().map(|a| a.name.clone()).collect(),
        art: song
            .art
            .largest()
            .filter(|url| !riff_api::models::is_resource_url(url))
            .map(str::to_owned),
    })
}

fn track_path(id: &str) -> String {
    if id.is_empty() {
        return NO_TRACK.to_owned();
    }
    let mut element = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            element.push(byte as char);
        } else {
            element.push_str(&format!("_{byte:02x}"));
        }
    }
    format!("/dev/diegovsky/Riff/Track/{element}")
}

const JUMP_TOLERANCE_MICROS: u128 = 1_500_000;

pub fn is_jump(expected_micros: u128, reported_micros: u128, by_user: bool) -> bool {
    by_user || expected_micros.abs_diff(reported_micros) > JUMP_TOLERANCE_MICROS
}

fn loop_status(repeat: RepeatMode) -> LoopStatus {
    match repeat {
        RepeatMode::Off => LoopStatus::None,
        RepeatMode::Track => LoopStatus::Track,
        RepeatMode::Context => LoopStatus::Playlist,
    }
}

fn playback_status(status: events::PlaybackStatus) -> PlaybackStatus {
    match status {
        events::PlaybackStatus::Playing => PlaybackStatus::Playing,
        events::PlaybackStatus::Paused => PlaybackStatus::Paused,
        events::PlaybackStatus::Stopped => PlaybackStatus::Stopped,
    }
}

pub fn update_for(event: &PlaybackChanged, playback: &PlaybackState) -> MprisUpdate {
    let changed = event.changed;
    let state = &event.state;
    let mut update = queue_update(playback);
    if changed.contains(PlaybackFields::TRACK) {
        update.track = Some(track_meta(playback));
    }
    if changed.intersects(PlaybackFields::STATUS | PlaybackFields::TRACK) {
        update.status = Some(playback_status(state.status));
    }
    if changed.contains(PlaybackFields::REPEAT) {
        update.loop_status = Some(loop_status(state.repeat));
    }
    if changed.contains(PlaybackFields::SHUFFLE) {
        update.shuffled = Some(state.shuffle);
    }
    if changed.contains(PlaybackFields::POSITION) {
        update.position = Some(Position {
            micros: 1000 * (state.position as u128),
            by_user: event.seeked_by_user(),
        });
    }
    if changed.contains(PlaybackFields::VOLUME) {
        update.volume = Some(state.volume);
    }
    update
}

pub fn queue_update(playback: &PlaybackState) -> MprisUpdate {
    MprisUpdate {
        has_prev: playback.has_prev(),
        has_next: playback.has_next(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riff_core::events::{PlaybackSnapshot, PositionOrigin};

    fn event(changed: PlaybackFields, origin: Option<PositionOrigin>) -> PlaybackChanged {
        PlaybackChanged {
            changed,
            state: PlaybackSnapshot {
                position: 2_000,
                shuffle: true,
                ..Default::default()
            },
            position_origin: origin,
        }
    }

    #[test]
    fn track_ids_become_valid_object_paths() {
        assert_eq!(track_path("4uLU6hMC"), "/dev/diegovsky/Riff/Track/4uLU6hMC");
        assert_eq!(track_path("a:b-c"), "/dev/diegovsky/Riff/Track/a_3ab_2dc");
        assert_eq!(track_path(""), NO_TRACK);
    }

    #[test]
    fn different_track_ids_get_different_paths() {
        let ids = ["a:b", "a-b", "a_b", "a b", "a_3ab"];
        let paths: std::collections::HashSet<_> = ids.iter().map(|id| track_path(id)).collect();
        assert_eq!(paths.len(), ids.len());
        for path in paths {
            assert!(
                zbus::zvariant::ObjectPath::try_from(path.as_str()).is_ok(),
                "{path}"
            );
        }
    }

    #[test]
    fn only_jumps_are_seeks() {
        assert!(!is_jump(10_000_000, 10_400_000, false));
        assert!(is_jump(10_000_000, 30_000_000, false));
        assert!(is_jump(30_000_000, 10_000_000, false));
        assert!(is_jump(10_000_000, 10_000_000, true));
    }

    #[test]
    fn position_is_sent_in_microseconds() {
        let playback = PlaybackState::default();
        let update = update_for(
            &event(PlaybackFields::POSITION, Some(PositionOrigin::User)),
            &playback,
        );
        assert_eq!(
            update.position,
            Some(Position {
                micros: 2_000_000,
                by_user: true
            })
        );
        let update = update_for(
            &event(PlaybackFields::POSITION, Some(PositionOrigin::Player)),
            &playback,
        );
        assert_eq!(update.position.map(|p| p.by_user), Some(false));
    }

    #[test]
    fn only_changed_fields_are_sent() {
        let playback = PlaybackState::default();
        let update = update_for(&event(PlaybackFields::SHUFFLE, None), &playback);
        assert_eq!(update.shuffled, Some(true));
        assert!(update.track.is_none() && update.status.is_none() && update.position.is_none());
        let update = update_for(&event(PlaybackFields::TRACK, None), &playback);
        assert!(matches!(update.track, Some(None)));
        assert_eq!(update.status, Some(PlaybackStatus::Stopped));
        assert!(update.shuffled.is_none());
    }

    #[test]
    fn repeat_modes_map_to_loop_status() {
        assert_eq!(loop_status(RepeatMode::Off), LoopStatus::None);
        assert_eq!(loop_status(RepeatMode::Track), LoopStatus::Track);
        assert_eq!(loop_status(RepeatMode::Context), LoopStatus::Playlist);
    }
}
