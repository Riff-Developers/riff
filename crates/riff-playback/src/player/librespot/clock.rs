use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use librespot::core::SpotifyUri;
use librespot::playback::player::PlayerEvent;

pub(super) const POSITION_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Default)]
pub(super) struct PositionClock(Arc<Mutex<Option<Mark>>>);

#[derive(Clone)]
struct Mark {
    track: SpotifyUri,
    position_ms: u32,
    playing: bool,
    at: Instant,
}

impl PositionClock {
    pub(super) fn follow(&self, event: &PlayerEvent) {
        let mut mark = self.0.lock().unwrap();
        let playing = mark.as_ref().is_some_and(|m| m.playing);
        let (track, position_ms, playing) = match event {
            PlayerEvent::Playing {
                track_id,
                position_ms,
                ..
            }
            | PlayerEvent::PositionChanged {
                track_id,
                position_ms,
                ..
            } => (track_id, *position_ms, true),
            PlayerEvent::Paused {
                track_id,
                position_ms,
                ..
            } => (track_id, *position_ms, false),
            PlayerEvent::Seeked {
                track_id,
                position_ms,
                ..
            } => (track_id, *position_ms, playing),
            PlayerEvent::Stopped { .. } | PlayerEvent::EndOfTrack { .. } => {
                *mark = None;
                return;
            }
            _ => return,
        };
        *mark = Some(Mark {
            track: track.clone(),
            position_ms,
            playing,
            at: Instant::now(),
        });
    }

    pub(super) fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }

    pub(super) fn playing(&self) -> Option<bool> {
        self.0.lock().unwrap().as_ref().map(|mark| mark.playing)
    }

    pub(super) fn now(&self) -> Option<(SpotifyUri, u32)> {
        let mark = self.0.lock().unwrap().clone()?;
        let since = if mark.playing {
            mark.at.elapsed().min(POSITION_UPDATE_INTERVAL)
        } else {
            Duration::ZERO
        };
        Some((mark.track, mark.position_ms + since.as_millis() as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> SpotifyUri {
        SpotifyUri::from_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap()
    }

    #[test]
    fn the_clock_follows_the_player() {
        let clock = PositionClock::default();
        assert!(clock.now().is_none());
        clock.follow(&PlayerEvent::Paused {
            play_request_id: 1,
            track_id: track(),
            position_ms: 42_000,
        });
        assert_eq!(clock.now(), Some((track(), 42_000)));
        clock.follow(&PlayerEvent::Seeked {
            play_request_id: 1,
            track_id: track(),
            position_ms: 10_000,
        });
        assert_eq!(clock.now(), Some((track(), 10_000)));
        clock.follow(&PlayerEvent::Playing {
            play_request_id: 1,
            track_id: track(),
            position_ms: 10_000,
        });
        let (_, now) = clock.now().unwrap();
        assert!((10_000..=11_000).contains(&now));
        clock.follow(&PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: track(),
        });
        assert!(clock.now().is_none());
    }
}
