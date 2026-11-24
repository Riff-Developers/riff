use riff_api::models::{Device, RepeatMode, Track};
use riff_api::remote::{
    QueueEntry, RemoteCluster, RemoteCommand, RemoteContext, RemotePlayback, TransferPlayback,
};

use super::session::Request;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum CurrentDevice {
    #[default]
    ThisDevice,
    Other(Device),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackView {
    pub online: bool,
    pub current: CurrentDevice,
    pub others: Vec<Device>,
    pub this_device_listed: bool,
    pub playback: Option<Playing>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playing {
    pub track_id: Option<String>,
    pub track: Option<Track>,
    pub context: Option<RemoteContext>,
    pub is_playing: bool,
    pub position_ms: u32,
    pub duration_ms: Option<u32>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub next: Vec<UpNext>,
    pub volume_percent: Option<u8>,
    pub queue_revision: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpNext {
    pub track_id: String,
    pub queued: bool,
    pub track: Option<Track>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    Play {
        context: Option<RemoteContext>,
        track_ids: Vec<String>,
        track_id: Option<String>,
        index: Option<usize>,
        position_ms: u32,
        shuffle: Option<bool>,
    },
    Resume,
    Pause,
    Next,
    Previous,
    Seek(u32),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    SetVolume(u8),
    AddToQueue(String),
    SkipTo {
        index: usize,
        revision: String,
    },
    MoveNext {
        from: usize,
        to: usize,
        revision: String,
    },
    RemoveNext {
        indices: Vec<usize>,
        revision: String,
    },
    ClearQueued,
    SwitchTo(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Local(RemoteCommand),
    LocalVolume(u8),
    Send(Request),
    Select(Option<String>),
    ThisDeviceUnlisted,
    TakePlayback { otherwise: Box<Route> },
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub cluster: RemoteCluster,
    pub this_device_id: Option<String>,
    pub selected: Option<String>,
}

impl State {
    fn active(&self) -> Option<&str> {
        self.cluster.active_device_id.as_deref()
    }

    fn is_this_device(&self, id: &str) -> bool {
        self.this_device_id.as_deref() == Some(id)
    }

    pub fn target(&self) -> Option<String> {
        let id = self.active().or(self.selected.as_deref())?;
        (!self.is_this_device(id)).then(|| id.to_string())
    }

    fn device(&self, id: &str) -> Device {
        self.cluster
            .devices
            .iter()
            .find(|d| d.id == id)
            .cloned()
            .unwrap_or_else(|| Device {
                id: id.to_string(),
                ..Default::default()
            })
    }
}

pub fn view(state: &State, online: bool) -> PlaybackView {
    let target = state.target();
    let current = match &target {
        Some(id) => CurrentDevice::Other(state.device(id)),
        None => CurrentDevice::ThisDevice,
    };
    let others = state
        .cluster
        .devices
        .iter()
        .filter(|d| Some(&d.id) != target.as_ref() && !state.is_this_device(&d.id))
        .cloned()
        .collect();
    let this_device_listed = state
        .this_device_id
        .as_ref()
        .is_some_and(|id| state.cluster.devices.iter().any(|d| &d.id == id));

    let on_target = match (state.active(), &target) {
        (Some(active), Some(id)) => active == id,
        (Some(active), None) => state.is_this_device(active),
        (None, _) => false,
    };
    let volume_device = target.as_deref().or(state.this_device_id.as_deref());
    let volume_percent = volume_device
        .map(|id| state.device(id))
        .filter(|d| d.supports_volume)
        .and_then(|d| d.volume_percent);
    let playback = state
        .cluster
        .playback
        .as_ref()
        .filter(|_| on_target)
        .map(|p| Playing {
            track_id: p.track_id.clone(),
            track: None,
            context: p.context.clone(),
            is_playing: p.is_playing,
            position_ms: p.position_ms,
            duration_ms: p.duration_ms,
            shuffle: p.shuffle,
            repeat: p.repeat,
            next: p
                .next
                .iter()
                .filter_map(|e| {
                    Some(UpNext {
                        track_id: e.track_id.clone()?,
                        queued: e.queued,
                        track: None,
                    })
                })
                .collect(),
            volume_percent,
            queue_revision: p.queue_revision.clone(),
        });

    PlaybackView {
        online,
        current,
        others,
        this_device_listed,
        playback,
    }
}

pub fn plays_all_but_queue_edits(command: &RemoteCommand) -> bool {
    !matches!(
        command,
        RemoteCommand::AddToQueue { .. }
            | RemoteCommand::SetQueue { .. }
            | RemoteCommand::SkipTo(_)
    )
}

pub fn route(
    intent: Intent,
    state: &State,
    plays_here: impl Fn(&RemoteCommand) -> bool,
) -> Option<Route> {
    let target = state.target();
    let command = match intent {
        Intent::SwitchTo(to) => return switch(to, state),
        Intent::SetVolume(percent) => {
            return Some(match target {
                Some(to) => Route::Send(Request::Volume { to, percent }),
                None => Route::LocalVolume(percent),
            })
        }
        Intent::Play {
            context: Some(context),
            track_id,
            index,
            position_ms,
            shuffle,
            ..
        } => RemoteCommand::PlayContext {
            context,
            track_id,
            track_index: index,
            position_ms,
            paused: false,
            shuffle,
        },
        Intent::Play {
            context: None,
            track_ids,
            track_id,
            index,
            position_ms,
            shuffle,
        } => RemoteCommand::PlayTracks {
            index: index.or_else(|| {
                let id = track_id?;
                track_ids.iter().position(|t| *t == id)
            }),
            track_ids,
            position_ms,
            paused: false,
            shuffle,
        },
        Intent::Resume => RemoteCommand::Resume,
        Intent::Pause => RemoteCommand::Pause,
        Intent::Next => RemoteCommand::SkipNext,
        Intent::Previous => RemoteCommand::SkipPrevious,
        Intent::Seek(position) => RemoteCommand::SeekTo(position),
        Intent::SetShuffle(shuffle) => RemoteCommand::SetShuffle(shuffle),
        Intent::SetRepeat(mode) => RemoteCommand::SetRepeat(mode),
        Intent::AddToQueue(track_id) => RemoteCommand::AddToQueue { track_id },
        Intent::SkipTo { index, revision } => {
            let playback = state.cluster.playback.as_ref()?;
            if playback.queue_revision != revision {
                return None;
            }
            let entry = playback
                .next
                .iter()
                .filter(|e| e.track_id.is_some())
                .nth(index)?;
            RemoteCommand::SkipTo(entry.clone())
        }
        Intent::MoveNext { from, to, revision } => {
            set_queue(state, &revision, |next| move_entry(next, from, to))?
        }
        Intent::RemoveNext { indices, revision } => {
            set_queue(state, &revision, |next| remove_entries(next, &indices))?
        }
        Intent::ClearQueued => set_queue(state, current_revision(state)?, |next| {
            let before = next.len();
            next.retain(|e| !e.queued);
            next.len() != before
        })?,
    };
    let provider_only = !plays_here(&command);
    Some(match target {
        Some(to) => Route::Send(Request::Command { to, command }),
        None if provider_only => match state.this_device_id.clone() {
            Some(to) => Route::Send(Request::Command { to, command }),
            None => Route::ThisDeviceUnlisted,
        },
        None => Route::Local(command),
    })
}

fn current_revision(state: &State) -> Option<&str> {
    Some(&state.cluster.playback.as_ref()?.queue_revision)
}

fn set_queue(
    state: &State,
    revision: &str,
    edit: impl FnOnce(&mut Vec<QueueEntry>) -> bool,
) -> Option<RemoteCommand> {
    let RemotePlayback {
        next,
        previous,
        queue_revision,
        ..
    } = state.cluster.playback.as_ref()?;
    if queue_revision != revision {
        return None;
    }
    let mut next = next.clone();
    edit(&mut next).then(|| RemoteCommand::SetQueue {
        next,
        previous: previous.clone(),
        revision: queue_revision.clone(),
    })
}

fn shown(next: &[QueueEntry]) -> Vec<usize> {
    (0..next.len())
        .filter(|&i| next[i].track_id.is_some())
        .collect()
}

fn move_entry(next: &mut Vec<QueueEntry>, from: usize, to: usize) -> bool {
    let Some(&source) = shown(next).get(from) else {
        return false;
    };
    if from == to {
        return false;
    }
    let entry = next.remove(source);
    let rest = shown(next);
    let at = match rest.get(to) {
        Some(&i) => i,
        None => rest.last().map_or(next.len(), |&i| i + 1),
    };
    next.insert(at, entry);
    true
}

fn remove_entries(next: &mut Vec<QueueEntry>, indices: &[usize]) -> bool {
    let positions = shown(next);
    let mut drop: Vec<usize> = indices
        .iter()
        .filter_map(|&i| positions.get(i).copied())
        .collect();
    drop.sort_unstable();
    drop.dedup();
    for &i in drop.iter().rev() {
        next.remove(i);
    }
    !drop.is_empty()
}

fn switch(to: Option<String>, state: &State) -> Option<Route> {
    let to = to.filter(|id| !state.is_this_device(id));
    if to == state.target() {
        return None;
    }
    if state.active().is_none() {
        return Some(Route::Select(to));
    }
    let transfer = |to: String| {
        Route::Send(Request::Transfer {
            to,
            playback: TransferPlayback::Keep,
        })
    };
    match to {
        Some(id) => is_listed(&id, state).then(|| transfer(id)),
        None => {
            let otherwise = match state.this_device_id.clone() {
                Some(id) if is_listed(&id, state) => transfer(id),
                _ => Route::ThisDeviceUnlisted,
            };
            Some(Route::TakePlayback {
                otherwise: Box::new(otherwise),
            })
        }
    }
}

fn is_listed(id: &str, state: &State) -> bool {
    state.cluster.devices.iter().any(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(intent: Intent, state: &State) -> Option<Route> {
        super::route(intent, state, plays_all_but_queue_edits)
    }
    use riff_api::models::DeviceKind;
    use riff_api::remote::RemotePlayback;

    fn device(id: &str) -> Device {
        Device {
            id: id.to_string(),
            label: id.to_string(),
            kind: DeviceKind::Phone,
            volume_percent: Some(40),
            supports_volume: true,
        }
    }

    fn state(active: Option<&str>) -> State {
        State {
            cluster: RemoteCluster {
                active_device_id: active.map(str::to_string),
                devices: vec![device("own"), device("phone"), device("tv")],
                playback: active.map(|_| RemotePlayback {
                    track_id: Some("t1".to_string()),
                    is_playing: true,
                    ..Default::default()
                }),
            },
            this_device_id: Some("own".to_string()),
            selected: None,
        }
    }

    #[test]
    fn nothing_active_is_this_device() {
        let view = view(&state(None), true);
        assert_eq!(view.current, CurrentDevice::ThisDevice);
        assert_eq!(view.others, [device("phone"), device("tv")]);
        assert!(view.this_device_listed);
        assert_eq!(view.playback, None);
    }

    #[test]
    fn shows_the_active_device() {
        let view = view(&state(Some("phone")), true);
        assert_eq!(view.current, CurrentDevice::Other(device("phone")));
        assert_eq!(view.others, [device("tv")]);
        let playing = view.playback.unwrap();
        assert_eq!(playing.track_id.as_deref(), Some("t1"));
        assert_eq!(playing.volume_percent, Some(40));
    }

    #[test]
    fn receiver_playing_is_this_device() {
        let view = view(&state(Some("own")), true);
        assert_eq!(view.current, CurrentDevice::ThisDevice);
        assert!(view.playback.is_some());
    }

    #[test]
    fn controls_go_to_the_device_playing() {
        let remote = route(Intent::Pause, &state(Some("phone")));
        assert_eq!(
            remote,
            Some(Route::Send(Request::Command {
                to: "phone".to_string(),
                command: RemoteCommand::Pause
            }))
        );
        assert_eq!(
            route(Intent::Pause, &state(Some("own"))),
            Some(Route::Local(RemoteCommand::Pause))
        );
        assert_eq!(
            route(Intent::SetVolume(20), &state(None)),
            Some(Route::LocalVolume(20))
        );
    }

    #[test]
    fn queueing_here_goes_through_the_provider() {
        let add = || Intent::AddToQueue("t2".to_string());
        assert_eq!(
            route(add(), &state(Some("own"))),
            Some(Route::Send(Request::Command {
                to: "own".to_string(),
                command: RemoteCommand::AddToQueue {
                    track_id: "t2".to_string()
                }
            }))
        );
        let mut unlisted = state(None);
        unlisted.this_device_id = None;
        assert_eq!(route(add(), &unlisted), Some(Route::ThisDeviceUnlisted));
    }

    #[test]
    fn what_this_device_doesnt_play_goes_through_the_provider() {
        let plays = |command: &RemoteCommand| matches!(command, RemoteCommand::Pause);
        let state = state(Some("own"));
        assert_eq!(
            super::route(Intent::Pause, &state, plays),
            Some(Route::Local(RemoteCommand::Pause))
        );
        assert_eq!(
            super::route(Intent::Next, &state, plays),
            Some(Route::Send(Request::Command {
                to: "own".to_string(),
                command: RemoteCommand::SkipNext
            }))
        );
    }

    fn entry(id: &str, queued: bool) -> QueueEntry {
        QueueEntry::for_test((id != "-").then_some(id), queued)
    }

    fn queued_state() -> State {
        let mut state = state(Some("own"));
        let playback = state.cluster.playback.as_mut().unwrap();
        playback.next = vec![
            entry("q1", true),
            entry("q2", true),
            entry("c1", false),
            entry("-", false),
            entry("c2", false),
        ];
        playback.queue_revision = "7".to_string();
        state
    }

    fn edited(route: Option<Route>) -> Vec<String> {
        let Some(Route::Send(Request::Command {
            to,
            command: RemoteCommand::SetQueue { next, revision, .. },
        })) = route
        else {
            panic!("expected a queue edit, got {route:?}");
        };
        assert_eq!(to, "own");
        assert_eq!(revision, "7");
        next.iter()
            .map(|e| e.track_id.clone().unwrap_or_else(|| "-".to_string()))
            .collect()
    }

    #[test]
    fn up_next_leaves_hidden_entries_out() {
        let playing = view(&queued_state(), true).playback.unwrap();
        let ids: Vec<_> = playing.next.iter().map(|n| n.track_id.as_str()).collect();
        assert_eq!(ids, ["q1", "q2", "c1", "c2"]);
        assert!(playing.next[1].queued && !playing.next[2].queued);
    }

    #[test]
    fn queue_edits_keep_hidden_entries_in_place() {
        let state = queued_state();
        let revision = || "7".to_string();
        let moved = |from, to| {
            edited(route(
                Intent::MoveNext {
                    from,
                    to,
                    revision: revision(),
                },
                &state,
            ))
        };
        assert_eq!(moved(0, 2), ["q2", "c1", "-", "q1", "c2"]);
        assert_eq!(moved(3, 0), ["c2", "q1", "q2", "c1", "-"]);
        assert_eq!(moved(0, 3), ["q2", "c1", "-", "c2", "q1"]);
        let unmoved =
            |from, to, revision: String| route(Intent::MoveNext { from, to, revision }, &state);
        assert_eq!(unmoved(1, 1, revision()), None);
        assert_eq!(unmoved(9, 0, revision()), None);
        assert_eq!(unmoved(0, 2, "6".to_string()), None, "a moved-on queue");

        let removed = edited(route(
            Intent::RemoveNext {
                indices: vec![3, 0, 3],
                revision: revision(),
            },
            &state,
        ));
        assert_eq!(removed, ["q2", "c1", "-"]);
        assert_eq!(
            edited(route(Intent::ClearQueued, &state)),
            ["c1", "-", "c2"]
        );
    }

    #[test]
    fn skipping_ahead_targets_the_entry() {
        let skip = |index, revision: &str| {
            route(
                Intent::SkipTo {
                    index,
                    revision: revision.to_string(),
                },
                &queued_state(),
            )
        };
        assert_eq!(
            skip(3, "7"),
            Some(Route::Send(Request::Command {
                to: "own".to_string(),
                command: RemoteCommand::SkipTo(entry("c2", false))
            }))
        );
        assert_eq!(skip(4, "7"), None);
        assert_eq!(skip(3, "6"), None, "a moved-on queue");
    }

    #[test]
    fn switching_moves_playback() {
        assert_eq!(
            route(
                Intent::SwitchTo(Some("tv".to_string())),
                &state(Some("phone"))
            ),
            Some(Route::Send(Request::Transfer {
                to: "tv".to_string(),
                playback: TransferPlayback::Keep
            }))
        );
        assert_eq!(
            route(Intent::SwitchTo(None), &state(Some("phone"))),
            Some(Route::TakePlayback {
                otherwise: Box::new(Route::Send(Request::Transfer {
                    to: "own".to_string(),
                    playback: TransferPlayback::Keep
                }))
            })
        );
        assert_eq!(route(Intent::SwitchTo(None), &state(Some("own"))), None);
    }

    #[test]
    fn switching_while_nothing_plays_selects() {
        let mut idle = state(None);
        assert_eq!(
            route(Intent::SwitchTo(Some("tv".to_string())), &idle),
            Some(Route::Select(Some("tv".to_string())))
        );
        idle.selected = Some("tv".to_string());
        assert_eq!(
            view(&idle, true).current,
            CurrentDevice::Other(device("tv"))
        );
        assert_eq!(
            route(Intent::Resume, &idle),
            Some(Route::Send(Request::Command {
                to: "tv".to_string(),
                command: RemoteCommand::Resume
            }))
        );
    }

    #[test]
    fn unlisted_receiver_cant_be_moved_to() {
        let mut unlisted = state(Some("phone"));
        unlisted.this_device_id = None;
        assert_eq!(
            route(Intent::SwitchTo(None), &unlisted),
            Some(Route::TakePlayback {
                otherwise: Box::new(Route::ThisDeviceUnlisted)
            })
        );
    }
}
