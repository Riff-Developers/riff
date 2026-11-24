use rand::seq::SliceRandom;
use riff_api::models::RepeatMode;
use riff_api::remote::{QueueEntry, RemoteContext, RemotePlayback};

use crate::position::Position;

const LISTED: usize = 100;
const RESTART_WITHIN_MS: u32 = 3000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Play {
    Track {
        track_id: String,
        position_ms: u32,
        paused: bool,
    },
    Resume,
    Pause,
    Seek(u32),
    Stop,
}

#[derive(Clone, Debug, Default)]
pub struct Start {
    pub context: Option<RemoteContext>,
    pub tracks: Vec<String>,
    pub track_id: Option<String>,
    pub index: Option<usize>,
    pub shuffle: Option<bool>,
    pub position_ms: u32,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum From {
    Context(usize),
    Queued,
}

#[derive(Clone, Debug)]
struct Current {
    track_id: String,
    from: From,
}

#[derive(Debug, Default)]
pub struct LocalQueue {
    context: Option<RemoteContext>,
    tracks: Vec<String>,
    order: Vec<usize>,
    at: Option<usize>,
    queued: Vec<(u64, String)>,
    next_key: u64,
    current: Option<Current>,
    shuffle: bool,
    repeat: RepeatMode,
    position: Position,
    revision: u64,
    stopped: bool,
}

impl LocalQueue {
    pub fn load(&mut self, start: Start) -> Play {
        let Start {
            context,
            tracks,
            track_id,
            index,
            shuffle,
            position_ms,
            paused,
        } = start;
        if let Some(shuffle) = shuffle {
            self.shuffle = shuffle;
        }
        let start = track_id
            .and_then(|id| tracks.iter().position(|t| *t == id))
            .or(index.filter(|&i| i < tracks.len()));
        self.context = context;
        self.tracks = tracks;
        if self.tracks.is_empty() {
            self.order.clear();
            self.at = None;
            self.current = None;
            self.edited();
            return self.stop();
        }
        let start = start.unwrap_or_else(|| {
            if self.shuffle {
                rand::random_range(0..self.tracks.len())
            } else {
                0
            }
        });
        self.order = (0..self.tracks.len()).collect();
        if self.shuffle {
            self.shuffle_after(start);
            self.at = Some(0);
        } else {
            self.at = Some(start);
        }
        self.edited();
        self.play_context(self.at.unwrap(), position_ms, paused)
    }

    fn shuffle_after(&mut self, first: usize) {
        let mut rest: Vec<usize> = (0..self.tracks.len()).filter(|&i| i != first).collect();
        rest.shuffle(&mut rand::rng());
        self.order = std::iter::once(first).chain(rest).collect();
    }

    fn play_context(&mut self, at: usize, position_ms: u32, paused: bool) -> Play {
        self.at = Some(at);
        let track_id = self.tracks[self.order[at]].clone();
        self.start(track_id, From::Context(at), position_ms, paused)
    }

    fn start(&mut self, track_id: String, from: From, position_ms: u32, paused: bool) -> Play {
        self.stopped = false;
        self.current = Some(Current {
            track_id: track_id.clone(),
            from,
        });
        self.position.set(position_ms, !paused);
        self.edited();
        Play::Track {
            track_id,
            position_ms,
            paused,
        }
    }

    fn stop(&mut self) -> Play {
        self.position.set(0, false);
        self.stopped = true;
        Play::Stop
    }

    pub fn player_stopped(&mut self) {
        self.position.run(false);
        self.stopped = true;
    }

    fn reload(&mut self, position_ms: u32, paused: bool) -> Option<Play> {
        let current = self.current.clone()?;
        Some(self.start(current.track_id, current.from, position_ms, paused))
    }

    fn edited(&mut self) {
        self.revision += 1;
    }

    pub fn is_loaded(&self) -> bool {
        self.current.is_some()
    }

    pub fn resume(&mut self) -> Option<Play> {
        if self.current.is_none() || self.position.is_running() {
            return None;
        }
        if self.stopped {
            return self.reload(self.position.now(), false);
        }
        self.position.run(true);
        Some(Play::Resume)
    }

    pub fn pause(&mut self) -> Option<Play> {
        if !self.position.is_running() {
            return None;
        }
        self.position.run(false);
        Some(Play::Pause)
    }

    pub fn seek(&mut self, position_ms: u32) -> Option<Play> {
        self.current.as_ref()?;
        if self.stopped {
            return self.reload(position_ms, true);
        }
        self.position.set(position_ms, self.position.is_running());
        Some(Play::Seek(position_ms))
    }

    pub fn next(&mut self, ended: bool) -> Play {
        let Some(current) = self.current.clone() else {
            return Play::Stop;
        };
        if ended && self.repeat == RepeatMode::Track {
            return self.start(current.track_id, current.from, 0, false);
        }
        if !self.queued.is_empty() {
            let (_, track_id) = self.queued.remove(0);
            return self.start(track_id, From::Queued, 0, false);
        }
        let next = self.upcoming_from();
        if next < self.order.len() {
            return self.play_context(next, 0, false);
        }
        if self.repeat != RepeatMode::Off && !self.order.is_empty() {
            if self.shuffle {
                let first = self.order[rand::random_range(0..self.order.len())];
                self.shuffle_after(first);
            }
            return self.play_context(0, 0, false);
        }
        self.stop()
    }

    pub fn previous(&mut self) -> Play {
        let Some(current) = self.current.clone() else {
            return Play::Stop;
        };
        let paused = !self.position.is_running();
        if self.position.now() > RESTART_WITHIN_MS {
            return self.seek(0).unwrap_or(Play::Stop);
        }
        match (current.from, self.at) {
            (From::Queued, Some(at)) => self.play_context(at, 0, paused),
            (From::Context(at), _) if at > 0 => self.play_context(at - 1, 0, paused),
            _ => self.seek(0).unwrap_or(Play::Stop),
        }
    }

    pub fn add(&mut self, track_id: String) {
        self.next_key += 1;
        self.queued.push((self.next_key, track_id));
        self.edited();
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        if shuffle == self.shuffle {
            return;
        }
        self.shuffle = shuffle;
        let Some(at) = self.at else {
            return;
        };
        let current = self.order[at];
        if shuffle {
            let mut rest = self.order.split_off(at + 1);
            rest.shuffle(&mut rand::rng());
            self.order.extend(rest);
        } else {
            self.order = (0..self.tracks.len()).collect();
            self.at = Some(current);
            if let Some(Current {
                from: From::Context(at),
                ..
            }) = self.current.as_mut()
            {
                *at = current;
            }
        }
        self.edited();
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn set_next(&mut self, next: &[QueueEntry], revision: &str) -> Option<()> {
        if revision != self.revision.to_string() {
            return None;
        }
        let listed = self.listed_context_len();
        let from = self.upcoming_from();
        let mut queued: Vec<(u64, String)> = vec![];
        let mut upcoming = vec![];
        let mut positions = vec![];
        for entry in next {
            match parse_key(entry.key()?)? {
                Key::Queued(key) => {
                    if queued.iter().any(|(k, _)| *k == key) {
                        return None;
                    }
                    let (_, track_id) = self.queued.iter().find(|(k, _)| *k == key)?;
                    queued.push((key, track_id.clone()));
                }
                Key::Context(position) => {
                    if !(from..from + listed).contains(&position) || positions.contains(&position) {
                        return None;
                    }
                    positions.push(position);
                    upcoming.push(self.order[position]);
                }
            }
        }
        self.queued = queued;
        let tail = self.order.split_off((from + listed).min(self.order.len()));
        self.order.truncate(from);
        self.order.extend(upcoming);
        self.order.extend(tail);
        self.edited();
        Some(())
    }

    pub fn skip_to(&mut self, entry: &QueueEntry) -> Option<Play> {
        match parse_key(entry.key()?)? {
            Key::Queued(key) => {
                let index = self.queued.iter().position(|(k, _)| *k == key)?;
                let (_, track_id) = self.queued.remove(index);
                self.queued.drain(..index);
                Some(self.start(track_id, From::Queued, 0, false))
            }
            Key::Context(position) => {
                if position >= self.order.len() {
                    return None;
                }
                self.queued.clear();
                Some(self.play_context(position, 0, false))
            }
        }
    }

    pub(super) fn track_id(&self) -> Option<&str> {
        self.current.as_ref().map(|c| c.track_id.as_str())
    }

    pub fn sync(&mut self, position_ms: u32, playing: bool) {
        self.position.set(position_ms, playing);
    }

    fn upcoming_from(&self) -> usize {
        self.at.map_or(0, |at| at + 1)
    }

    fn listed_context_len(&self) -> usize {
        let from = self.upcoming_from();
        let room = LISTED.saturating_sub(self.queued.len());
        self.order.len().saturating_sub(from).min(room)
    }

    pub fn playback(&self) -> Option<RemotePlayback> {
        let current = self.current.as_ref()?;
        let queued = self
            .queued
            .iter()
            .map(|(key, id)| QueueEntry::keyed(id, true, format!("q:{key}")));
        let from = self.upcoming_from();
        let context = (from..from + self.listed_context_len()).map(|position| {
            let id = &self.tracks[self.order[position]];
            QueueEntry::keyed(id, false, format!("c:{position}"))
        });
        Some(RemotePlayback {
            track_id: Some(current.track_id.clone()),
            context: self.context.clone(),
            is_playing: self.position.is_running(),
            position_ms: self.position.now(),
            duration_ms: None,
            shuffle: self.shuffle,
            repeat: self.repeat,
            next: queued.chain(context).take(LISTED).collect(),
            previous: vec![],
            queue_revision: self.revision.to_string(),
        })
    }
}

enum Key {
    Queued(u64),
    Context(usize),
}

fn parse_key(key: &str) -> Option<Key> {
    if let Some(key) = key.strip_prefix("q:") {
        key.parse().ok().map(Key::Queued)
    } else {
        key.strip_prefix("c:")?.parse().ok().map(Key::Context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(range: std::ops::Range<usize>) -> Vec<String> {
        range.map(|i| format!("t{i}")).collect()
    }

    fn up_next(queue: &LocalQueue) -> Vec<String> {
        let playback = queue.playback().unwrap();
        playback
            .next
            .into_iter()
            .filter_map(|e| e.track_id)
            .collect()
    }

    fn playing(queue: &LocalQueue) -> &str {
        queue.track_id().unwrap()
    }

    fn album() -> Option<RemoteContext> {
        Some(RemoteContext::Album("a".into()))
    }

    #[test]
    fn plays_a_context_from_a_track() {
        let mut queue = LocalQueue::default();
        let play = queue.load(Start {
            context: album(),
            tracks: ids(0..4),
            track_id: Some("t1".into()),
            index: None,
            shuffle: None,
            position_ms: 0,
            paused: false,
        });
        assert_eq!(
            play,
            Play::Track {
                track_id: "t1".into(),
                position_ms: 0,
                paused: false
            }
        );
        assert_eq!(up_next(&queue), ["t2", "t3"]);
        assert_eq!(queue.playback().unwrap().context, album());
        assert!(matches!(queue.next(true), Play::Track { track_id, .. } if track_id == "t2"));
        queue.next(true);
        assert_eq!(queue.next(true), Play::Stop);
        assert_eq!(playing(&queue), "t3", "the last track stays loaded");
    }

    #[test]
    fn playing_after_the_end_starts_the_last_track_again() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            tracks: ids(0..1),
            ..Default::default()
        });
        assert_eq!(queue.next(true), Play::Stop);
        assert_eq!(
            queue.resume(),
            Some(Play::Track {
                track_id: "t0".into(),
                position_ms: 0,
                paused: false
            })
        );
        assert_eq!(queue.pause(), Some(Play::Pause));
        assert_eq!(
            queue.resume(),
            Some(Play::Resume),
            "a paused track just resumes"
        );

        queue.resume();
        queue.player_stopped();
        assert!(
            matches!(queue.resume(), Some(Play::Track { track_id, .. }) if track_id == "t0"),
            "a player that stopped by itself loads the track again"
        );

        assert_eq!(queue.next(true), Play::Stop);
        assert_eq!(
            queue.seek(5000),
            Some(Play::Track {
                track_id: "t0".into(),
                position_ms: 5000,
                paused: true
            })
        );
    }

    #[test]
    fn queued_tracks_play_first_and_back_goes_to_the_context() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: None,
            tracks: ids(0..3),
            track_id: None,
            index: Some(0),
            shuffle: None,
            position_ms: 0,
            paused: false,
        });
        queue.add("q1".into());
        queue.add("q2".into());
        assert_eq!(up_next(&queue), ["q1", "q2", "t1", "t2"]);
        assert!(queue.playback().unwrap().next[0].queued);
        queue.next(false);
        assert_eq!(playing(&queue), "q1");
        queue.previous();
        assert_eq!(playing(&queue), "t0");
        queue.next(false);
        assert_eq!(playing(&queue), "q2", "what's still queued comes next");
        queue.next(false);
        assert_eq!(playing(&queue), "t1");
    }

    #[test]
    fn repeat_plays_again() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: None,
            tracks: ids(0..2),
            track_id: None,
            index: None,
            shuffle: None,
            position_ms: 0,
            paused: false,
        });
        queue.set_repeat(RepeatMode::Track);
        queue.next(true);
        assert_eq!(playing(&queue), "t0");
        queue.next(false);
        assert_eq!(playing(&queue), "t1", "skipping still moves on");
        queue.set_repeat(RepeatMode::Context);
        queue.next(true);
        assert_eq!(playing(&queue), "t0");
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_plays_each_once() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: None,
            tracks: ids(0..20),
            track_id: Some("t5".into()),
            index: None,
            shuffle: Some(true),
            position_ms: 0,
            paused: false,
        });
        assert_eq!(playing(&queue), "t5");
        let mut next = up_next(&queue);
        assert_eq!(next.len(), 19);
        next.sort();
        let mut rest = ids(0..20);
        rest.retain(|id| id != "t5");
        rest.sort();
        assert_eq!(next, rest);

        queue.set_shuffle(false);
        assert_eq!(playing(&queue), "t5");
        assert_eq!(up_next(&queue)[0], "t6");
    }

    #[test]
    fn edits_and_skips_use_the_listed_entries() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: album(),
            tracks: ids(0..5),
            track_id: None,
            index: None,
            shuffle: None,
            position_ms: 0,
            paused: false,
        });
        queue.add("q".into());
        let playback = queue.playback().unwrap();
        let next = playback.next.clone();
        let edited = vec![
            next[3].clone(),
            next[0].clone(),
            next[1].clone(),
            next[4].clone(),
        ];
        assert!(queue.set_next(&edited, &playback.queue_revision).is_some());
        assert_eq!(up_next(&queue), ["q", "t3", "t1", "t4"]);
        assert!(
            queue.set_next(&edited, &playback.queue_revision).is_none(),
            "a stale version is refused"
        );

        let playback = queue.playback().unwrap();
        let t1 = playback.next[2].clone();
        assert!(
            matches!(queue.skip_to(&t1), Some(Play::Track { track_id, .. }) if track_id == "t1")
        );
        assert_eq!(up_next(&queue), ["t4"]);
    }

    #[test]
    fn edits_that_repeat_or_reach_past_the_listing_are_refused() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: album(),
            tracks: ids(0..5),
            index: Some(2),
            ..Start::default()
        });
        let playback = queue.playback().unwrap();
        let t3 = playback.next[0].clone();
        let repeated = vec![t3.clone(), t3];
        assert!(queue
            .set_next(&repeated, &playback.queue_revision)
            .is_none());
        let played = vec![QueueEntry::keyed("t0", false, "c:0".into())];
        assert!(queue.set_next(&played, &playback.queue_revision).is_none());
        assert_eq!(up_next(&queue), ["t3", "t4"]);
    }

    #[test]
    fn clearing_the_queue_keeps_every_context_track() {
        let mut queue = LocalQueue::default();
        queue.load(Start {
            context: album(),
            tracks: ids(0..300),
            index: Some(0),
            ..Start::default()
        });
        for i in 0..10 {
            queue.add(format!("q{i}"));
        }
        let playback = queue.playback().unwrap();
        let context: Vec<_> = playback
            .next
            .iter()
            .filter(|e| e.key().is_some_and(|k| k.starts_with("c:")))
            .cloned()
            .collect();
        assert_eq!(context.len(), 90);
        assert!(queue.set_next(&context, &playback.queue_revision).is_some());
        assert_eq!(queue.order.len(), 300);
        let mut order = queue.order.clone();
        order.sort_unstable();
        assert_eq!(order, (0..300).collect::<Vec<_>>());
        assert_eq!(up_next(&queue), ids(1..101));
    }

    #[test]
    fn nothing_to_play_stops() {
        let mut queue = LocalQueue::default();
        assert_eq!(
            queue.load(Start {
                context: None,
                tracks: vec![],
                track_id: None,
                index: None,
                shuffle: None,
                position_ms: 0,
                paused: false,
            }),
            Play::Stop
        );
        assert!(!queue.is_loaded());
        assert_eq!(queue.next(true), Play::Stop);
        assert!(queue.resume().is_none());
        assert!(queue.playback().is_none());
    }
}
