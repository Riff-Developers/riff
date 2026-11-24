use std::time::{Duration, Instant};

use serde_json::Value;

use super::protocol::{self, Event, Playback, Position};

const RESUME_WINDOW: Duration = Duration::from_secs(2 * 60);
const TAKE_BACK_WAIT: Duration = Duration::from_secs(3);
const TAKE_BACK_ATTEMPTS: u32 = 3;
const RESYNC_WINDOW: Duration = Duration::from_secs(30);
const RESYNC_TOLERANCE_MS: u64 = 1_500;

#[derive(Clone, Debug, PartialEq)]
struct Mark {
    track: Option<String>,
    position_ms: u64,
    at_ms: u64,
    speed: f64,
    playing: bool,
}

impl Mark {
    fn position_at(&self, now_ms: u64) -> u64 {
        let since = now_ms.saturating_sub(self.at_ms) as f64 * self.speed;
        self.position_ms + since.max(0.0) as u64
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LeftOff {
    track: String,
    position_ms: u64,
    playing: bool,
    lost_at: Instant,
}

#[derive(Debug)]
struct TakingBack {
    left_off: LeftOff,
    started: Instant,
    asked: Instant,
    attempts: u32,
}

impl TakingBack {
    fn expected(&self, now: Instant) -> u64 {
        let since = if self.left_off.playing {
            now.duration_since(self.started).as_millis() as u64
        } else {
            0
        };
        self.left_off.position_ms + since
    }
}

#[derive(Debug, Default)]
pub(super) struct Recovery {
    mark: Option<Mark>,
    left_off: Option<LeftOff>,
    taking_back: Option<TakingBack>,
}

impl Recovery {
    pub fn follow(&mut self, event: &Event, now_ms: u64) {
        match event {
            Event::PlaybackState(Playback {
                status,
                item,
                position,
                ..
            })
            | Event::PlaybackChanged(Playback {
                status,
                item,
                position,
                ..
            }) => {
                if *status == protocol::Status::Idle {
                    self.mark = None;
                    return;
                }
                let playing = status.playing();
                let previous = self.mark.take();
                let track = item
                    .as_ref()
                    .map(|item| item.uri.clone())
                    .or_else(|| previous.as_ref().and_then(|mark| mark.track.clone()));
                let same_track = previous.as_ref().is_some_and(|mark| mark.track == track);
                let mut mark = match position {
                    Some(position) => from_position(position, playing, now_ms),
                    None => Mark {
                        position_ms: previous
                            .as_ref()
                            .filter(|_| same_track)
                            .map_or(0, |mark| mark.position_at(now_ms)),
                        at_ms: now_ms,
                        speed: if playing { 1.0 } else { 0.0 },
                        ..from_position(&Position::default(), playing, now_ms)
                    },
                };
                mark.track = track;
                self.mark = Some(mark);
            }
            Event::PositionSync { position } => {
                let Some(previous) = self.mark.take() else {
                    return;
                };
                self.mark = Some(Mark {
                    track: previous.track,
                    ..from_position(position, previous.playing, now_ms)
                });
            }
            _ => {}
        }
    }

    pub fn lost(&mut self, active: bool, now: Instant, now_ms: u64) {
        if let Some(taking) = self.taking_back.take() {
            self.left_off = Some(taking.left_off);
        }
        let mark = self.mark.take();
        if !active {
            return;
        }
        let Some(mark) = mark else { return };
        let Some(track) = mark.track.clone() else {
            return;
        };
        let left_off = LeftOff {
            track,
            position_ms: mark.position_at(now_ms),
            playing: mark.playing,
            lost_at: now,
        };
        info!(
            "Soloist went away at {}ms into {} (playing: {})",
            left_off.position_ms, left_off.track, left_off.playing
        );
        self.left_off = Some(left_off);
    }

    pub fn forget(&mut self) {
        *self = Self::default();
    }

    pub fn signed_in(&mut self, now: Instant) -> Vec<Value> {
        let Some(left_off) = self.left_off.take() else {
            return vec![];
        };
        let gone = now.duration_since(left_off.lost_at);
        if gone > RESUME_WINDOW {
            info!("Soloist was gone for {gone:?}; not taking playback back");
            return vec![];
        }
        info!("Taking playback back after Soloist was gone for {gone:?}");
        self.taking_back = Some(TakingBack {
            left_off,
            started: now,
            asked: now,
            attempts: 1,
        });
        vec![protocol::activate()]
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.taking_back
            .as_ref()
            .map(|taking| taking.asked + TAKE_BACK_WAIT)
    }

    pub fn timed_out(&mut self, now: Instant) -> Vec<Value> {
        let Some(taking) = self.taking_back.as_mut() else {
            return vec![];
        };
        if taking.attempts >= TAKE_BACK_ATTEMPTS
            || now.duration_since(taking.started) > RESYNC_WINDOW
        {
            warn!("Playback didn't come back to Soloist; giving up on it");
            self.taking_back = None;
            return vec![];
        }
        taking.attempts += 1;
        taking.asked = now;
        warn!(
            "Playback didn't come back to Soloist yet; asking again ({})",
            taking.attempts
        );
        vec![protocol::activate()]
    }

    pub fn settle(&mut self, active: bool, now: Instant, now_ms: u64) -> Vec<Value> {
        if !active || self.taking_back.is_none() {
            return vec![];
        }
        let Some(mark) = self.mark.clone() else {
            return vec![];
        };
        let Some(track) = mark.track.as_deref() else {
            return vec![];
        };
        let taking = self.taking_back.take().unwrap();
        if track != taking.left_off.track {
            info!("Playback came back on another track; leaving it there");
            return vec![];
        }
        let mut commands = Vec::new();
        let expected = taking.expected(now);
        let position_ms = mark.position_at(now_ms);
        if expected.abs_diff(position_ms) > RESYNC_TOLERANCE_MS {
            info!(
                "Playback came back at {position_ms}ms; moving it to {expected}ms, where it left off"
            );
            commands.push(protocol::seek(expected));
        }
        match (taking.left_off.playing, mark.playing) {
            (true, false) => commands.push(protocol::play()),
            (false, true) => commands.push(protocol::pause()),
            _ => {}
        }
        commands
    }
}

fn from_position(position: &Position, playing: bool, now_ms: u64) -> Mark {
    Mark {
        track: None,
        position_ms: position.position_ms,
        at_ms: position.timestamp_ms.unwrap_or(now_ms),
        speed: position.speed.unwrap_or(if playing { 1.0 } else { 0.0 }),
        playing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::soloist::protocol::{Item, Status};

    const TRACK: &str = "spotify:track:3KvxPIAPSH7SHiovnx4znN";
    const START_MS: u64 = 1_800_000_000_000;

    fn state(status: Status, track: &str, position_ms: u64, at_ms: u64) -> Event {
        Event::PlaybackState(Playback {
            status,
            item: Some(Item {
                uri: track.to_string(),
            }),
            position: Some(Position {
                position_ms,
                timestamp_ms: Some(at_ms),
                speed: Some(if status.playing() { 1.0 } else { 0.0 }),
            }),
            is_active: Some(true),
        })
    }

    fn names(commands: &[Value]) -> Vec<String> {
        commands
            .iter()
            .map(|c| match c["position_ms"].as_u64() {
                Some(at) => format!("seek {at}"),
                None => c["command"].as_str().unwrap().to_string(),
            })
            .collect()
    }

    fn lost_while_playing(gone: Duration) -> (Recovery, Instant) {
        let lost = Instant::now();
        let mut recovery = Recovery::default();
        recovery.follow(&state(Status::Playing, TRACK, 50_000, START_MS), START_MS);
        recovery.lost(true, lost, START_MS + 10_000);
        (recovery, lost + gone)
    }

    #[test]
    fn a_new_track_without_a_position_starts_at_its_beginning() {
        let mut recovery = Recovery::default();
        recovery.follow(&state(Status::Playing, TRACK, 180_000, START_MS), START_MS);
        let next = Event::PlaybackChanged(Playback {
            status: Status::Playing,
            item: Some(Item {
                uri: "spotify:track:next".to_string(),
            }),
            position: None,
            is_active: Some(true),
        });
        recovery.follow(&next, START_MS + 1_000);
        let mark = recovery.mark.clone().unwrap();
        assert_eq!(mark.track.as_deref(), Some("spotify:track:next"));
        assert_eq!(mark.position_at(START_MS + 1_000), 0);
    }

    #[test]
    fn picks_up_where_it_was_playing() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(20));
        assert_eq!(names(&recovery.signed_in(back)), ["activate"]);
        let now_ms = START_MS + 40_000;
        recovery.follow(&state(Status::Paused, TRACK, 52_000, now_ms), now_ms);
        assert_eq!(
            names(&recovery.settle(true, back, now_ms)),
            ["seek 60000", "play"]
        );
        assert!(recovery.settle(true, back, now_ms).is_empty());
        assert_eq!(recovery.deadline(), None);
    }

    #[test]
    fn stays_paused_if_it_was() {
        let lost = Instant::now();
        let mut recovery = Recovery::default();
        recovery.follow(&state(Status::Paused, TRACK, 30_000, START_MS), START_MS);
        recovery.lost(true, lost, START_MS + 60_000);
        let back = lost + Duration::from_secs(5);
        recovery.signed_in(back);
        let now_ms = START_MS + 70_000;
        recovery.follow(&state(Status::Playing, TRACK, 30_500, now_ms), now_ms);
        assert_eq!(names(&recovery.settle(true, back, now_ms)), ["pause"]);
    }

    #[test]
    fn close_enough_is_left_alone() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.signed_in(back);
        let now_ms = START_MS + 20_000;
        recovery.follow(&state(Status::Playing, TRACK, 60_800, now_ms), now_ms);
        assert!(recovery.settle(true, back, now_ms).is_empty());
    }

    #[test]
    fn a_long_absence_is_not_picked_up() {
        let (mut recovery, back) = lost_while_playing(RESUME_WINDOW + Duration::from_secs(1));
        assert!(recovery.signed_in(back).is_empty());
        assert_eq!(recovery.deadline(), None);
    }

    #[test]
    fn nothing_is_picked_up_unless_it_was_the_active_device() {
        let mut recovery = Recovery::default();
        recovery.follow(&state(Status::Playing, TRACK, 0, START_MS), START_MS);
        recovery.lost(false, Instant::now(), START_MS);
        assert!(recovery.signed_in(Instant::now()).is_empty());
    }

    #[test]
    fn another_track_is_left_playing() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.signed_in(back);
        recovery.follow(
            &state(Status::Playing, "spotify:track:other", 0, START_MS),
            START_MS,
        );
        assert!(recovery.settle(true, back, START_MS).is_empty());
        assert_eq!(recovery.deadline(), None);
    }

    #[test]
    fn waits_for_it_to_be_the_active_device() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.signed_in(back);
        recovery.follow(&state(Status::Paused, TRACK, 0, START_MS), START_MS);
        assert!(recovery.settle(false, back, START_MS).is_empty());
        assert!(recovery.deadline().is_some());
    }

    #[test]
    fn asks_again_then_gives_up() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.signed_in(back);
        let first = recovery.deadline().unwrap();
        assert_eq!(names(&recovery.timed_out(first)), ["activate"]);
        let second = recovery.deadline().unwrap();
        assert_eq!(second, first + TAKE_BACK_WAIT);
        assert_eq!(names(&recovery.timed_out(second)), ["activate"]);
        assert!(recovery.timed_out(recovery.deadline().unwrap()).is_empty());
        assert_eq!(recovery.deadline(), None);
    }

    #[test]
    fn changes_without_a_position_carry_it_on() {
        let mut recovery = Recovery::default();
        recovery.follow(&state(Status::Playing, TRACK, 10_000, START_MS), START_MS);
        let paused = Event::PlaybackChanged(Playback {
            status: Status::Paused,
            item: None,
            position: None,
            is_active: None,
        });
        recovery.follow(&paused, START_MS + 5_000);
        let mark = recovery.mark.clone().unwrap();
        assert_eq!(mark.track.as_deref(), Some(TRACK));
        assert_eq!(mark.position_at(START_MS + 60_000), 15_000);
        recovery.follow(
            &Event::PositionSync {
                position: Position {
                    position_ms: 15_200,
                    timestamp_ms: None,
                    speed: None,
                },
            },
            START_MS + 6_000,
        );
        let mark = recovery.mark.clone().unwrap();
        assert_eq!(mark.track.as_deref(), Some(TRACK));
        assert_eq!(mark.position_ms, 15_200);
    }

    #[test]
    fn going_away_again_meanwhile_keeps_where_it_was() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.signed_in(back);
        recovery.lost(false, back + Duration::from_secs(1), START_MS);
        let again = back + Duration::from_secs(10);
        assert_eq!(names(&recovery.signed_in(again)), ["activate"]);
        let (mut late, back) = lost_while_playing(Duration::from_secs(5));
        late.signed_in(back);
        late.lost(false, back, START_MS);
        assert!(late.signed_in(back + RESUME_WINDOW).is_empty());
    }

    #[test]
    fn logging_out_forgets_it() {
        let (mut recovery, back) = lost_while_playing(Duration::from_secs(5));
        recovery.forget();
        assert!(recovery.signed_in(back).is_empty());
    }
}
