mod devices;
mod start;

use std::collections::HashMap;

use riff_api::models::{RepeatMode, Track};

use crate::events::{
    DeviceFields, DeviceRequest, DevicesChanged, ErrorKind, Event, PlaybackChanged, PlaybackFields,
    PlaybackSnapshot, PlaybackStatus, PositionOrigin, QueueChanged,
};
use crate::play_queue::{EntryKey, QueueRole, QueueRow, QueueSection, SongsSource};
use crate::position::Position;
use crate::remote::{CurrentDevice, Failure, Intent, PlaybackView, PlayerReport, Playing};

pub use devices::{DeviceEntry, DeviceList};
pub use start::{PlayContext, PlayRequest};

const UNMUTE_FALLBACK_VOLUME: f64 = 0.5;

#[derive(Debug, Default)]
pub struct PlaybackState {
    shown: PlaybackView,
    seek_position: Position,
    shuffle: bool,
    repeat: RepeatMode,
    volume: Option<f64>,
    muted_volume: Option<f64>,
    rows: Vec<QueueRow>,
    context_names: HashMap<SongsSource, String>,
    skip_explicit: bool,
    explicit_filter_locked: bool,
    placing: Option<Placing>,
}

#[derive(Debug)]
struct Placing {
    track_id: String,
    at: usize,
    queued_before: usize,
}

impl PlaybackState {
    fn playing(&self) -> Option<&Playing> {
        self.shown.playback.as_ref()
    }

    pub fn queue_rows(&self) -> &[QueueRow] {
        &self.rows
    }

    pub fn view_keys(&self) -> Vec<EntryKey> {
        self.rows.iter().map(|row| row.key).collect()
    }

    pub fn view_key(&self, index: usize) -> Option<EntryKey> {
        self.rows.get(index).map(|row| row.key)
    }

    pub fn upcoming_keys(&self) -> Vec<EntryKey> {
        self.rows
            .iter()
            .map(|row| row.key)
            .filter(|key| *key != EntryKey::Current)
            .collect()
    }

    pub fn view_track(&self, key: EntryKey) -> Option<Track> {
        let row = self.rows.iter().find(|row| row.key == key)?;
        Some(row.track.clone())
    }

    pub fn still_holds(&self, key: EntryKey, track_id: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.key == key && row.track.rri.id == track_id)
    }

    pub fn queue_is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn is_playing(&self) -> bool {
        self.seek_position.is_running() && self.playing().is_some()
    }

    pub fn status(&self) -> PlaybackStatus {
        match (self.playing(), self.seek_position.is_running()) {
            (None, _) => PlaybackStatus::Stopped,
            (Some(_), true) => PlaybackStatus::Playing,
            (Some(_), false) => PlaybackStatus::Paused,
        }
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            status: self.status(),
            track: self.current_song_id(),
            position: self.seek_position.now(),
            shuffle: self.shuffle,
            repeat: self.repeat,
            volume: self.volume.unwrap_or(0.0),
            skip_explicit: self.skip_explicit,
            local: self.shown.current == CurrentDevice::ThisDevice,
        }
    }

    fn adjust_volume(&mut self, delta: f64) -> Vec<Event> {
        match self.volume {
            Some(volume) => self.set_volume((volume + delta).clamp(0.0, 1.0)),
            None => vec![],
        }
    }

    fn toggle_mute(&mut self) -> Vec<Event> {
        match self.volume {
            None => vec![],
            Some(volume) if volume > 0.0 => {
                self.muted_volume = Some(volume);
                self.set_volume(0.0)
            }
            Some(_) => {
                let volume = self.muted_volume.take().unwrap_or(UNMUTE_FALLBACK_VOLUME);
                self.set_volume(volume)
            }
        }
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffle
    }

    pub fn explicit_filter_locked(&self) -> bool {
        self.explicit_filter_locked
    }

    pub fn skip_explicit(&self) -> bool {
        self.skip_explicit
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub fn current_source(&self) -> Option<SongsSource> {
        self.playing()?
            .context
            .as_ref()
            .map(SongsSource::from_remote)
    }

    pub fn current_key(&self) -> Option<EntryKey> {
        self.playing()?.track_id.as_ref().map(|_| EntryKey::Current)
    }

    pub fn current_song_id(&self) -> Option<String> {
        self.playing()?.track_id.clone()
    }

    pub fn current_song(&self) -> Option<Track> {
        self.playing()?.track.clone()
    }

    pub fn has_next(&self) -> bool {
        self.playing().is_some_and(|p| !p.next.is_empty())
    }

    pub fn next_song(&self) -> Option<Track> {
        self.playing()?.next.first()?.track.clone()
    }

    pub fn has_prev(&self) -> bool {
        self.current_song_id().is_some()
    }

    pub fn is_movable(&self, key: EntryKey) -> bool {
        matches!(key, EntryKey::Next(i) if i < self.upcoming_len())
    }

    pub fn can_move_entry(&self, key: EntryKey, target: EntryKey, after: bool) -> bool {
        self.queues_by_dropping(key, target, after).is_some()
            || self
                .move_positions(key, target, after)
                .is_some_and(|(from, to)| from != to)
    }

    fn role(&self, key: EntryKey) -> Option<QueueRole> {
        self.rows
            .iter()
            .find(|row| row.key == key)
            .map(|row| row.role)
    }

    fn queued_len(&self) -> usize {
        self.playing()
            .map_or(0, |p| p.next.iter().take_while(|next| next.queued).count())
    }

    fn queues_by_dropping(
        &self,
        key: EntryKey,
        target: EntryKey,
        after: bool,
    ) -> Option<(String, usize)> {
        let (EntryKey::Next(from), EntryKey::Next(target)) = (key, target) else {
            return None;
        };
        if self.role(key)? != QueueRole::Context
            || self.role(EntryKey::Next(target))? != QueueRole::Queued
        {
            return None;
        }
        let track_id = self.playing()?.next.get(from)?.track_id.clone();
        Some((track_id, if after { target + 1 } else { target }))
    }

    fn queue_at(&mut self, track_id: String, at: usize) -> Vec<Event> {
        let queued_before = self.queued_len();
        self.placing = (at < queued_before).then(|| Placing {
            track_id: track_id.clone(),
            at,
            queued_before,
        });
        vec![Self::request(Intent::AddToQueue(track_id))]
    }

    fn place_dropped(&mut self) -> Vec<Event> {
        let Some(placing) = self.placing.take() else {
            return vec![];
        };
        let queued = self.queued_len();
        if queued == placing.queued_before {
            self.placing = Some(placing);
            return vec![];
        }
        let last = queued.wrapping_sub(1);
        let added = self
            .playing()
            .and_then(|p| p.next.get(last))
            .is_some_and(|next| next.queued && next.track_id == placing.track_id);
        if queued != placing.queued_before + 1 || !added {
            return vec![];
        }
        vec![Self::request(Intent::MoveNext {
            from: last,
            to: placing.at,
            revision: self.queue_revision(),
        })]
    }

    pub fn device_list(&self) -> DeviceList {
        DeviceList::from_view(&self.shown)
    }

    fn upcoming_len(&self) -> usize {
        self.playing().map_or(0, |p| p.next.len())
    }

    fn move_positions(
        &self,
        key: EntryKey,
        target: EntryKey,
        after: bool,
    ) -> Option<(usize, usize)> {
        let (EntryKey::Next(from), EntryKey::Next(target)) = (key, target) else {
            return None;
        };
        let len = self.upcoming_len();
        if from >= len || target >= len {
            return None;
        }
        let to = match (from < target, after) {
            (true, true) => target,
            (true, false) => target - 1,
            (false, true) => target + 1,
            (false, false) => target,
        };
        Some((from, to))
    }

    fn refresh_rows(&mut self) {
        let context = QueueSection::Context(
            self.current_source()
                .and_then(|source| self.context_names.get(&source).cloned()),
        );
        self.rows = match self.playing() {
            None => vec![],
            Some(playing) => {
                let current = playing.track.clone().map(|track| QueueRow {
                    key: EntryKey::Current,
                    track,
                    role: QueueRole::Current,
                    section: QueueSection::NowPlaying,
                });
                let next = playing.next.iter().enumerate().filter_map(|(i, next)| {
                    let (role, section) = if next.queued {
                        (QueueRole::Queued, QueueSection::Queued)
                    } else {
                        (QueueRole::Context, context.clone())
                    };
                    Some(QueueRow {
                        key: EntryKey::Next(i),
                        track: next.track.clone()?,
                        role,
                        section,
                    })
                });
                current.into_iter().chain(next).collect()
            }
        };
    }

    fn changed(&self, changed: PlaybackFields) -> Event {
        self.event(changed, None)
    }

    fn seeked(&self, origin: PositionOrigin) -> Event {
        self.event(PlaybackFields::POSITION, Some(origin))
    }

    fn event(&self, changed: PlaybackFields, position_origin: Option<PositionOrigin>) -> Event {
        PlaybackChanged {
            changed,
            state: self.snapshot(),
            position_origin,
        }
        .into()
    }

    fn request(intent: Intent) -> Event {
        DeviceRequest::Control(intent).into()
    }

    fn show(&mut self, view: PlaybackView) -> Vec<Event> {
        let before = std::mem::replace(&mut self.shown, view);
        let (old, new) = (before.playback.as_ref(), self.shown.playback.clone());
        let mut events = vec![];

        let mut devices = DeviceFields::empty();
        if before.current != self.shown.current {
            devices |= DeviceFields::CURRENT;
        }
        if (&before.others, before.online, before.this_device_listed)
            != (
                &self.shown.others,
                self.shown.online,
                self.shown.this_device_listed,
            )
        {
            devices |= DeviceFields::AVAILABLE;
        }
        if !devices.is_empty() {
            events.push(DevicesChanged { changed: devices }.into());
        }

        let new = new.as_ref();
        let mut fields = PlaybackFields::empty();
        if old.map(|p| (&p.track_id, &p.track)) != new.map(|p| (&p.track_id, &p.track)) {
            fields |= PlaybackFields::TRACK;
        }
        if old.map(|p| &p.context) != new.map(|p| &p.context) {
            fields |= PlaybackFields::CONTEXT;
        }
        if fields.contains(PlaybackFields::TRACK)
            || old.map(|p| (&p.track, &p.next)) != new.map(|p| (&p.track, &p.next))
        {
            self.refresh_rows();
            events.push(QueueChanged.into());
        }

        match new.cloned() {
            None => {
                if self.seek_position.is_running() || old.is_some() {
                    fields |= PlaybackFields::STATUS;
                }
                self.seek_position.set(0, false);
            }
            Some(playing) => {
                if playing.is_playing != self.seek_position.is_running() || old.is_none() {
                    fields |= PlaybackFields::STATUS;
                }
                if playing.shuffle != self.shuffle {
                    self.shuffle = playing.shuffle;
                    fields |= PlaybackFields::SHUFFLE;
                }
                if playing.repeat != self.repeat {
                    self.repeat = playing.repeat;
                    fields |= PlaybackFields::REPEAT;
                }
                if let Some(percent) = playing.volume_percent {
                    let volume = percent as f64 / 100.0;
                    if self.volume.is_none_or(|v| (volume - v).abs() >= 0.01) {
                        self.volume = Some(volume);
                        fields |= PlaybackFields::VOLUME;
                    }
                }
                self.seek_position
                    .set(playing.position_ms, playing.is_playing);
            }
        }
        if !fields.is_empty() {
            events.push(self.changed(fields));
        }
        if new.is_some() {
            events.push(self.seeked(PositionOrigin::Player));
        }
        events.extend(self.place_dropped());
        events
    }

    fn set_playing(&mut self, playing: bool) -> Vec<Event> {
        if self.playing().is_none() {
            return if playing {
                vec![Self::request(Intent::Resume)]
            } else {
                vec![]
            };
        }
        if playing == self.seek_position.is_running() {
            return vec![];
        }
        self.seek_position.run(playing);
        let intent = if playing {
            Intent::Resume
        } else {
            Intent::Pause
        };
        vec![self.changed(PlaybackFields::STATUS), Self::request(intent)]
    }

    fn set_repeat(&mut self, mode: RepeatMode) -> Vec<Event> {
        self.repeat = mode;
        vec![
            self.changed(PlaybackFields::REPEAT),
            Self::request(Intent::SetRepeat(mode)),
        ]
    }

    fn set_shuffled(&mut self, shuffled: bool) -> Vec<Event> {
        self.shuffle = shuffled;
        vec![
            self.changed(PlaybackFields::SHUFFLE),
            Self::request(Intent::SetShuffle(shuffled)),
        ]
    }

    fn seek_to(&mut self, pos: u32) -> Vec<Event> {
        self.seek_position.set(pos, self.seek_position.is_running());
        vec![
            self.seeked(PositionOrigin::User),
            Self::request(Intent::Seek(pos)),
        ]
    }

    fn set_volume(&mut self, volume: f64) -> Vec<Event> {
        if self.volume == Some(volume) {
            return vec![];
        }
        self.volume = Some(volume);
        let percent = (volume * 100.0).round() as u8;
        vec![
            self.changed(PlaybackFields::VOLUME),
            Self::request(Intent::SetVolume(percent)),
        ]
    }

    fn set_skip_explicit(&mut self, skip: bool) -> Vec<Event> {
        if self.skip_explicit == skip {
            return vec![];
        }
        self.skip_explicit = skip;
        vec![self.changed(PlaybackFields::SKIP_EXPLICIT)]
    }

    pub fn dequeue(&self, entries: &[(EntryKey, String)]) -> Vec<Event> {
        let positions: Vec<usize> = entries
            .iter()
            .filter(|(key, track_id)| self.still_holds(*key, track_id))
            .filter_map(|(key, _)| match key {
                EntryKey::Next(i) => Some(*i),
                EntryKey::Current => None,
            })
            .collect();
        if positions.is_empty() {
            return vec![];
        }
        vec![Self::request(Intent::RemoveNext {
            indices: positions,
            revision: self.queue_revision(),
        })]
    }

    pub(crate) fn queue_revision(&self) -> String {
        self.playing()
            .map(|playing| playing.queue_revision.clone())
            .unwrap_or_default()
    }

    pub fn update(&mut self, action: PlaybackAction) -> Vec<Event> {
        match action {
            PlaybackAction::TogglePlay => self.set_playing(!self.is_playing()),
            PlaybackAction::Play => self.set_playing(true),
            PlaybackAction::Pause | PlaybackAction::Stop => self.set_playing(false),
            PlaybackAction::ToggleRepeat => self.set_repeat(match self.repeat {
                RepeatMode::Track => RepeatMode::Off,
                RepeatMode::Context => RepeatMode::Track,
                RepeatMode::Off => RepeatMode::Context,
            }),
            PlaybackAction::SetRepeatMode(mode) if self.repeat != mode => self.set_repeat(mode),
            PlaybackAction::SetRepeatMode(_) => vec![],
            PlaybackAction::ToggleShuffle => self.set_shuffled(!self.shuffle),
            PlaybackAction::SetShuffle(shuffle) if self.shuffle != shuffle => {
                self.set_shuffled(shuffle)
            }
            PlaybackAction::SetShuffle(_) => vec![],
            PlaybackAction::SetSkipExplicit(skip) => {
                self.set_skip_explicit(skip || self.explicit_filter_locked)
            }
            PlaybackAction::SetExplicitFilterLocked(locked) => {
                self.explicit_filter_locked = locked;
                if locked {
                    self.set_skip_explicit(true)
                } else {
                    vec![]
                }
            }
            PlaybackAction::Next => vec![Self::request(Intent::Next)],
            PlaybackAction::Previous => vec![Self::request(Intent::Previous)],
            PlaybackAction::PlayFrom(_) => vec![],
            PlaybackAction::PlayEntry(key) => match key {
                EntryKey::Next(index) => vec![Self::request(Intent::SkipTo {
                    index,
                    revision: self.queue_revision(),
                })],
                EntryKey::Current => self.seek_to(0),
            },
            PlaybackAction::ClearQueued => vec![Self::request(Intent::ClearQueued)],
            PlaybackAction::RemoveEntries(entries) => self.dequeue(&entries),
            PlaybackAction::MoveEntry {
                key,
                track_id,
                target,
                after,
            } => {
                if !self.still_holds(key, &track_id) {
                    return vec![];
                }
                if let Some((track_id, at)) = self.queues_by_dropping(key, target, after) {
                    return self.queue_at(track_id, at);
                }
                match self.move_positions(key, target, after) {
                    Some((from, to)) if from != to => {
                        vec![Self::request(Intent::MoveNext {
                            from,
                            to,
                            revision: self.queue_revision(),
                        })]
                    }
                    _ => vec![],
                }
            }
            PlaybackAction::SetContextName(source, name) => {
                let playing = self.current_source().as_ref() == Some(&source);
                self.context_names.insert(source, name);
                if playing {
                    self.refresh_rows();
                    vec![QueueChanged.into()]
                } else {
                    vec![]
                }
            }
            PlaybackAction::Seek(pos) => self.seek_to(pos),
            PlaybackAction::SeekBy(delta) => {
                let Some(duration) = self
                    .playing()
                    .and_then(|p| p.duration_ms.or(p.track.as_ref().map(|t| t.duration_ms)))
                else {
                    return vec![];
                };
                let target = (self.seek_position.now() as i64 + delta as i64).max(0);
                if target >= duration as i64 {
                    vec![Self::request(Intent::Next)]
                } else {
                    self.seek_to(target as u32)
                }
            }
            PlaybackAction::SetVolume(volume) => self.set_volume(volume),
            PlaybackAction::AdjustVolume(delta) => self.adjust_volume(delta),
            PlaybackAction::ToggleMute => self.toggle_mute(),
            PlaybackAction::TransferTo(id) => vec![Self::request(Intent::SwitchTo(id))],
            PlaybackAction::SetThisDeviceId(id) => vec![DeviceRequest::ThisDevice(id).into()],
            PlaybackAction::PlayerReport(report) => {
                vec![DeviceRequest::PlayerReport(report).into()]
            }
            PlaybackAction::Show(view) => self.show(*view),
            PlaybackAction::Failed(failure) => vec![ErrorKind::Device(failure).into()],
        }
    }
}

#[derive(Clone, Debug)]
pub enum PlaybackAction {
    TogglePlay,
    Play,
    Pause,
    Stop,
    SetRepeatMode(RepeatMode),
    SetSkipExplicit(bool),
    SetExplicitFilterLocked(bool),
    ToggleRepeat,
    ToggleShuffle,
    SetShuffle(bool),
    Seek(u32),
    SeekBy(i32),
    PlayFrom(PlayRequest),
    SetVolume(f64),
    AdjustVolume(f64),
    ToggleMute,
    Next,
    Previous,
    ClearQueued,
    SetContextName(SongsSource, String),
    PlayEntry(EntryKey),
    RemoveEntries(Vec<(EntryKey, String)>),
    MoveEntry {
        key: EntryKey,
        track_id: String,
        target: EntryKey,
        after: bool,
    },
    TransferTo(Option<String>),
    SetThisDeviceId(Option<String>),
    PlayerReport(PlayerReport),
    Show(Box<PlaybackView>),
    Failed(Failure),
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_playing_from(
    source: Option<SongsSource>,
    current: Track,
    queued: &[Track],
    next: &[Track],
) -> PlaybackAction {
    use crate::remote::UpNext;
    let up_next = |tracks: &[Track], queued: bool| -> Vec<UpNext> {
        tracks
            .iter()
            .map(|track| UpNext {
                track_id: track.rri.id.clone(),
                queued,
                track: Some(track.clone()),
            })
            .collect()
    };
    let mut next_up = up_next(queued, true);
    next_up.extend(up_next(next, false));
    PlaybackAction::Show(Box::new(PlaybackView {
        online: true,
        playback: Some(Playing {
            track_id: Some(current.rri.id.clone()),
            track: Some(current),
            context: source.and_then(|s| s.remote_context(Some("me"))),
            is_playing: true,
            next: next_up,
            ..Default::default()
        }),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::UpNext;
    use crate::test_support::{requests, track};

    fn up_next(id: &str, queued: bool) -> UpNext {
        UpNext {
            track_id: id.to_string(),
            queued,
            track: Some(track(id)),
        }
    }

    fn view(current: &str, next: &[(&str, bool)]) -> PlaybackView {
        PlaybackView {
            online: true,
            playback: Some(Playing {
                track_id: Some(current.to_string()),
                track: Some(track(current)),
                context: Some(riff_api::remote::RemoteContext::Album("a".to_string())),
                is_playing: true,
                position_ms: 1000,
                duration_ms: Some(200_000),
                next: next
                    .iter()
                    .map(|(id, queued)| up_next(id, *queued))
                    .collect(),
                volume_percent: Some(40),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn show(state: &mut PlaybackState, view: PlaybackView) -> Vec<Event> {
        state.update(PlaybackAction::Show(Box::new(view)))
    }

    fn changed(events: &[Event]) -> PlaybackFields {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Playback(p) if p.position_origin.is_none() => Some(p.changed),
                _ => None,
            })
            .fold(PlaybackFields::empty(), |a, b| a | b)
    }

    fn row_ids(state: &PlaybackState) -> Vec<String> {
        state
            .queue_rows()
            .iter()
            .map(|r| r.track.rri.id.clone())
            .collect()
    }

    #[test]
    fn shows_what_the_device_reports() {
        let mut state = PlaybackState::default();
        let events = show(&mut state, view("t1", &[("q1", true), ("t2", false)]));
        assert!(changed(&events).contains(PlaybackFields::TRACK | PlaybackFields::STATUS));
        assert!(events.contains(&QueueChanged.into()));
        assert!(state.is_playing());
        assert_eq!(state.current_song_id().as_deref(), Some("t1"));
        assert_eq!(state.current_source(), Some(SongsSource::Album("a".into())));
        assert!(state.has_next());
        assert_eq!(row_ids(&state), ["t1", "q1", "t2"]);
        assert_eq!(
            state.upcoming_keys(),
            [EntryKey::Next(0), EntryKey::Next(1)]
        );

        let again = show(&mut state, view("t1", &[("q1", true), ("t2", false)]));
        assert!(matches!(
            &again[..],
            [Event::Playback(p)] if p.position_origin == Some(PositionOrigin::Player)
        ));

        let stopped = show(&mut state, PlaybackView::default());
        assert!(changed(&stopped).contains(PlaybackFields::TRACK | PlaybackFields::STATUS));
        assert_eq!(state.status(), PlaybackStatus::Stopped);
        assert!(state.queue_is_empty());
    }

    #[test]
    fn devices_changes_are_told() {
        let mut state = PlaybackState::default();
        let device = riff_api::models::Device {
            id: "phone".into(),
            label: "Phone".into(),
            kind: riff_api::models::DeviceKind::Phone,
            volume_percent: Some(30),
            supports_volume: true,
        };
        let elsewhere = PlaybackView {
            online: true,
            current: CurrentDevice::Other(device),
            ..Default::default()
        };
        let events = show(&mut state, elsewhere);
        assert!(events.contains(
            &DevicesChanged {
                changed: DeviceFields::CURRENT | DeviceFields::AVAILABLE
            }
            .into()
        ));
        assert!(!state.snapshot().local);
        assert_eq!(state.device_list().current_volume(), Some(30));
    }

    #[test]
    fn controls_become_requests() {
        let mut state = PlaybackState::default();
        show(&mut state, view("t1", &[("t2", false)]));

        let paused = state.update(PlaybackAction::TogglePlay);
        assert_eq!(requests(&paused), [Intent::Pause]);
        assert!(!state.is_playing());
        assert_eq!(
            requests(&state.update(PlaybackAction::Next)),
            [Intent::Next]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::SetVolume(0.25))),
            [Intent::SetVolume(25)]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::ToggleShuffle)),
            [Intent::SetShuffle(true)]
        );
        assert!(state.is_shuffled());
        assert!(requests(&state.update(PlaybackAction::SetShuffle(true))).is_empty());
        assert_eq!(
            requests(&state.update(PlaybackAction::SetShuffle(false))),
            [Intent::SetShuffle(false)]
        );
        assert!(!state.is_shuffled());
        assert_eq!(
            requests(&state.update(PlaybackAction::TransferTo(Some("tv".into())))),
            [Intent::SwitchTo(Some("tv".into()))]
        );
    }

    #[test]
    fn nothing_loaded_resumes_the_last_playback() {
        let mut state = PlaybackState::default();
        assert_eq!(
            requests(&state.update(PlaybackAction::TogglePlay)),
            [Intent::Resume]
        );
        assert!(requests(&state.update(PlaybackAction::Pause)).is_empty());
    }

    #[test]
    fn queue_edits_use_positions_up_next() {
        let mut state = PlaybackState::default();
        show(
            &mut state,
            view("t1", &[("q1", true), ("q2", true), ("t2", false)]),
        );
        let moved = state.update(PlaybackAction::MoveEntry {
            key: EntryKey::Next(0),
            track_id: "q1".to_string(),
            target: EntryKey::Next(2),
            after: true,
        });
        let revision = state.queue_revision();
        assert_eq!(
            requests(&moved),
            [Intent::MoveNext {
                from: 0,
                to: 2,
                revision: revision.clone()
            }]
        );
        let up = state.update(PlaybackAction::MoveEntry {
            key: EntryKey::Next(1),
            track_id: "q2".to_string(),
            target: EntryKey::Next(0),
            after: false,
        });
        assert_eq!(
            requests(&up),
            [Intent::MoveNext {
                from: 1,
                to: 0,
                revision: revision.clone()
            }]
        );
        assert!(!state.can_move_entry(EntryKey::Next(0), EntryKey::Next(1), false));

        let removed = state.update(PlaybackAction::RemoveEntries(vec![
            (EntryKey::Current, "t1".to_string()),
            (EntryKey::Next(1), "q2".to_string()),
            (EntryKey::Next(2), "moved-on".to_string()),
        ]));
        assert_eq!(
            requests(&removed),
            [Intent::RemoveNext {
                indices: vec![1],
                revision: revision.clone()
            }]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::PlayEntry(EntryKey::Next(2)))),
            [Intent::SkipTo { index: 2, revision }]
        );
    }

    #[test]
    fn queue_sections_name_the_context() {
        let mut state = PlaybackState::default();
        show(&mut state, view("t1", &[("q1", true), ("t2", false)]));
        let sections = |state: &PlaybackState| -> Vec<QueueSection> {
            state
                .queue_rows()
                .iter()
                .map(|r| r.section.clone())
                .collect()
        };
        assert_eq!(sections(&state)[2], QueueSection::Context(None));

        let events = state.update(PlaybackAction::SetContextName(
            SongsSource::Album("a".into()),
            "An Album".into(),
        ));
        assert_eq!(events, [QueueChanged.into()]);
        assert_eq!(
            sections(&state),
            [
                QueueSection::NowPlaying,
                QueueSection::Queued,
                QueueSection::Context(Some("An Album".into())),
            ]
        );
    }

    #[test]
    fn seek_by_is_relative_and_skips_past_the_end() {
        let mut state = PlaybackState::default();
        let mut paused = view("t1", &[]);
        paused.playback.as_mut().unwrap().is_playing = false;
        show(&mut state, paused);
        assert_eq!(
            requests(&state.update(PlaybackAction::SeekBy(5000))),
            [Intent::Seek(6000)]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::SeekBy(-60_000))),
            [Intent::Seek(0)]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::SeekBy(500_000))),
            [Intent::Next]
        );
    }

    #[test]
    fn mute_restores_the_volume() {
        let mut state = PlaybackState::default();
        show(&mut state, view("t1", &[]));
        assert_eq!(
            requests(&state.update(PlaybackAction::ToggleMute)),
            [Intent::SetVolume(0)]
        );
        assert_eq!(
            requests(&state.update(PlaybackAction::ToggleMute)),
            [Intent::SetVolume(40)]
        );
    }

    #[test]
    fn dropping_a_track_on_the_queue_queues_it_there() {
        let mut state = PlaybackState::default();
        let next = [("q1", true), ("q2", true), ("t2", false), ("t3", false)];
        show(&mut state, view("t1", &next));

        assert!(state.can_move_entry(EntryKey::Next(2), EntryKey::Next(1), true));
        let at_the_end = state.update(PlaybackAction::MoveEntry {
            key: EntryKey::Next(2),
            track_id: "t2".to_string(),
            target: EntryKey::Next(1),
            after: true,
        });
        assert_eq!(
            requests(&at_the_end),
            [Intent::AddToQueue("t2".to_string())]
        );
        let queued = [("q1", true), ("q2", true), ("t2", true), ("t2", false)];
        assert!(requests(&show(&mut state, view("t1", &queued))).is_empty());

        show(&mut state, view("t1", &next));
        let first = state.update(PlaybackAction::MoveEntry {
            key: EntryKey::Next(3),
            track_id: "t3".to_string(),
            target: EntryKey::Next(0),
            after: false,
        });
        assert_eq!(requests(&first), [Intent::AddToQueue("t3".to_string())]);
        let added = [
            ("q1", true),
            ("q2", true),
            ("t3", true),
            ("t2", false),
            ("t3", false),
        ];
        let placed = show(&mut state, view("t1", &added));
        assert_eq!(
            requests(&placed),
            [Intent::MoveNext {
                from: 2,
                to: 0,
                revision: state.queue_revision()
            }]
        );
        assert!(requests(&show(&mut state, view("t1", &added))).is_empty());
    }

    #[test]
    fn an_unknown_volume_is_left_alone() {
        let mut state = PlaybackState::default();
        let mut quiet = view("t1", &[]);
        quiet.playback.as_mut().unwrap().volume_percent = None;
        show(&mut state, quiet);
        assert!(requests(&state.update(PlaybackAction::AdjustVolume(0.05))).is_empty());
        assert!(requests(&state.update(PlaybackAction::ToggleMute)).is_empty());
        show(&mut state, view("t1", &[]));
        assert_eq!(
            requests(&state.update(PlaybackAction::AdjustVolume(0.05))),
            [Intent::SetVolume(45)]
        );
    }

    #[test]
    fn track_details_arriving_later_change_the_track() {
        let mut state = PlaybackState::default();
        let mut unresolved = view("t1", &[]);
        unresolved.playback.as_mut().unwrap().track = None;
        show(&mut state, unresolved);
        let events = show(&mut state, view("t1", &[]));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Playback(PlaybackChanged { changed, .. }) if changed.contains(PlaybackFields::TRACK)
        )));
    }

    #[test]
    fn locked_filter_forces_skipping() {
        let mut state = PlaybackState::default();
        state.update(PlaybackAction::SetExplicitFilterLocked(true));
        assert!(state.skip_explicit());
        state.update(PlaybackAction::SetSkipExplicit(false));
        assert!(state.skip_explicit());
    }

    #[test]
    fn failures_are_errors() {
        let mut state = PlaybackState::default();
        assert_eq!(
            state.update(PlaybackAction::Failed(Failure::Offline)),
            [ErrorKind::Device(Failure::Offline).into()]
        );
    }
}
