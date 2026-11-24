use std::sync::Arc;

use riff_api::models::{Device, DeviceKind};
use riff_api::remote::{RemoteCluster, RemoteCommand, RemoteContext};
use riff_api::{ApiService, DomainError, Load};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::local_queue::{LocalQueue, Play, Start};
use super::session::{Request, SessionEvent};
use super::LocalDevice;
use crate::play_queue::{playable_tracks, SongsSource};

pub const LOCAL_DEVICE_ID: &str = "this-device";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerReport {
    Playing { position_ms: u32 },
    Paused { position_ms: u32 },
    EndOfTrack { track_id: Option<String> },
    Unavailable { track_id: Option<String> },
    Stopped { track_id: Option<String> },
}

enum Input {
    Request(Request),
    Report(PlayerReport),
}

pub struct LocalSession {
    inputs: UnboundedSender<Input>,
}

impl LocalSession {
    pub fn start(
        api: Arc<ApiService>,
        player: Arc<dyn LocalDevice>,
        load: fn() -> Load,
    ) -> (Self, UnboundedReceiver<SessionEvent>) {
        let (inputs, input_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(api, player, load, input_rx, events));
        (Self { inputs }, event_rx)
    }

    pub fn request(&self, request: Request) {
        let _ = self.inputs.send(Input::Request(request));
    }

    pub fn report(&self, report: PlayerReport) {
        let _ = self.inputs.send(Input::Report(report));
    }
}

struct Run {
    api: Arc<ApiService>,
    player: Arc<dyn LocalDevice>,
    load: fn() -> Load,
    queue: LocalQueue,
    volume: Option<u8>,
}

async fn run(
    api: Arc<ApiService>,
    player: Arc<dyn LocalDevice>,
    load: fn() -> Load,
    mut inputs: UnboundedReceiver<Input>,
    events: UnboundedSender<SessionEvent>,
) {
    let mut run = Run {
        api,
        player,
        load,
        queue: LocalQueue::default(),
        volume: None,
    };
    let _ = events.send(SessionEvent::Online(true));
    let _ = events.send(SessionEvent::Cluster(run.cluster()));
    while let Some(input) = inputs.recv().await {
        match input {
            Input::Request(request) => {
                if let Err(error) = run.handle(request.clone()).await {
                    warn!("local playback request {request:?} failed: {error}");
                    let _ = events.send(SessionEvent::RequestFailed(error));
                }
            }
            Input::Report(report) => run.on_report(report),
        }
        let _ = events.send(SessionEvent::Cluster(run.cluster()));
    }
}

impl Run {
    async fn handle(&mut self, request: Request) -> Result<(), DomainError> {
        match request {
            Request::Command { to, command } if to == LOCAL_DEVICE_ID => {
                self.command(command).await
            }
            Request::Volume { to, percent } if to == LOCAL_DEVICE_ID => {
                self.volume = Some(percent);
                self.player.set_volume(percent);
                Ok(())
            }
            Request::Transfer { .. } => Err(DomainError::Unsupported("other devices")),
            Request::Command { .. } | Request::Volume { .. } => Err(DomainError::NotFound {
                resource: "device".to_string(),
            }),
        }
    }

    async fn command(&mut self, command: RemoteCommand) -> Result<(), DomainError> {
        let queue = &mut self.queue;
        let play = match command {
            RemoteCommand::Resume => queue.resume(),
            RemoteCommand::Pause => queue.pause(),
            RemoteCommand::SkipNext => Some(queue.next(false)),
            RemoteCommand::SkipPrevious => Some(queue.previous()),
            RemoteCommand::SeekTo(position_ms) => queue.seek(position_ms),
            RemoteCommand::SetShuffle(shuffle) => {
                queue.set_shuffle(shuffle);
                None
            }
            RemoteCommand::SetRepeat(repeat) => {
                queue.set_repeat(repeat);
                None
            }
            RemoteCommand::PlayContext {
                context,
                track_id,
                track_index,
                position_ms,
                paused,
                shuffle,
            } => {
                let tracks = context_tracks(&self.api, &context, (self.load)()).await?;
                Some(self.queue.load(Start {
                    context: Some(context),
                    tracks,
                    track_id,
                    index: track_index,
                    shuffle,
                    position_ms,
                    paused,
                }))
            }
            RemoteCommand::PlayTracks {
                track_ids,
                index,
                position_ms,
                paused,
                shuffle,
            } => Some(queue.load(Start {
                tracks: track_ids,
                index,
                shuffle,
                position_ms,
                paused,
                ..Default::default()
            })),
            RemoteCommand::AddToQueue { track_id } => {
                queue.add(track_id);
                None
            }
            RemoteCommand::SetQueue { next, revision, .. } => {
                queue.set_next(&next, &revision).ok_or_else(stale)?;
                None
            }
            RemoteCommand::SkipTo(entry) => Some(queue.skip_to(&entry).ok_or_else(stale)?),
        };
        if let Some(play) = play {
            self.play(play);
        }
        Ok(())
    }

    fn on_report(&mut self, report: PlayerReport) {
        let play = match report {
            PlayerReport::Playing { position_ms } => {
                self.queue.sync(position_ms, true);
                None
            }
            PlayerReport::Paused { position_ms } => {
                self.queue.sync(position_ms, false);
                None
            }
            PlayerReport::EndOfTrack { track_id } if self.is_current(&track_id) => {
                Some(self.queue.next(true))
            }
            PlayerReport::Unavailable { track_id } if self.is_current(&track_id) => {
                Some(self.queue.next(false))
            }
            PlayerReport::Stopped { track_id } if self.is_current(&track_id) => {
                self.queue.player_stopped();
                None
            }
            PlayerReport::EndOfTrack { .. }
            | PlayerReport::Unavailable { .. }
            | PlayerReport::Stopped { .. } => None,
        };
        if let Some(play) = play {
            self.play(play);
        }
    }

    fn is_current(&self, track_id: &Option<String>) -> bool {
        track_id
            .as_deref()
            .is_none_or(|id| self.queue.track_id() == Some(id))
    }

    fn play(&self, play: Play) {
        let command = match play {
            Play::Track {
                track_id,
                position_ms,
                paused,
            } => RemoteCommand::PlayTracks {
                track_ids: vec![track_id],
                index: Some(0),
                position_ms,
                paused,
                shuffle: Some(false),
            },
            Play::Resume => RemoteCommand::Resume,
            Play::Pause | Play::Stop => RemoteCommand::Pause,
            Play::Seek(position_ms) => RemoteCommand::SeekTo(position_ms),
        };
        self.player.command(command);
    }

    fn cluster(&self) -> RemoteCluster {
        let this_device = Device {
            id: LOCAL_DEVICE_ID.to_string(),
            label: String::new(),
            kind: DeviceKind::Computer,
            volume_percent: self.volume,
            supports_volume: true,
        };
        RemoteCluster {
            active_device_id: self.queue.is_loaded().then(|| LOCAL_DEVICE_ID.to_string()),
            devices: vec![this_device],
            playback: self.queue.playback(),
        }
    }
}

fn stale() -> DomainError {
    DomainError::Conflict("the queue changed".to_string())
}

async fn context_tracks(
    api: &ApiService,
    context: &RemoteContext,
    load: Load,
) -> Result<Vec<String>, DomainError> {
    let tracks = playable_tracks(api, &SongsSource::from_remote(context), load).await?;
    Ok(tracks.into_iter().map(|t| t.rri.id).collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use riff_api::remote::QueueEntry;

    #[derive(Default)]
    struct FakePlayer {
        commands: Mutex<Vec<RemoteCommand>>,
        volume: Mutex<Option<u8>>,
    }

    impl LocalDevice for FakePlayer {
        fn command(&self, command: RemoteCommand) {
            self.commands.lock().unwrap().push(command);
        }

        fn set_volume(&self, percent: u8) {
            *self.volume.lock().unwrap() = Some(percent);
        }
    }

    impl FakePlayer {
        fn last_track(&self) -> Option<String> {
            self.commands
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|c| match c {
                    RemoteCommand::PlayTracks { track_ids, .. } => Some(track_ids[0].clone()),
                    _ => None,
                })
        }
    }

    fn api() -> Arc<ApiService> {
        let tokens: Arc<dyn riff_api::TokenProvider> = Arc::new(|| None::<String>);
        Arc::new(riff_api::spotify_service(
            Arc::clone(&tokens),
            tokens,
            1024 * 1024,
            1024 * 1024,
        ))
    }

    fn command(command: RemoteCommand) -> Request {
        Request::Command {
            to: LOCAL_DEVICE_ID.to_string(),
            command,
        }
    }

    async fn cluster(events: &mut UnboundedReceiver<SessionEvent>) -> RemoteCluster {
        loop {
            match events.recv().await.expect("an event") {
                SessionEvent::Cluster(cluster) => return cluster,
                SessionEvent::Online(online) => assert!(online),
                SessionEvent::RequestFailed(error) => panic!("a request failed: {error}"),
            }
        }
    }

    #[tokio::test]
    async fn plays_tracks_one_by_one_on_this_device() {
        let player = Arc::new(FakePlayer::default());
        let (session, mut events) = LocalSession::start(api(), player.clone(), Load::background);
        let idle = cluster(&mut events).await;
        assert_eq!(idle.active_device_id, None);
        assert_eq!(idle.devices[0].id, LOCAL_DEVICE_ID);

        session.request(command(RemoteCommand::PlayTracks {
            track_ids: vec!["a".into(), "b".into()],
            index: None,
            position_ms: 0,
            paused: false,
            shuffle: None,
        }));
        let playing = cluster(&mut events).await;
        assert_eq!(playing.active_device_id.as_deref(), Some(LOCAL_DEVICE_ID));
        let playback = playing.playback.unwrap();
        assert_eq!(playback.track_id.as_deref(), Some("a"));
        assert_eq!(playback.next[0].track_id.as_deref(), Some("b"));
        assert_eq!(playback.next.len(), 1);
        assert_eq!(player.last_track().as_deref(), Some("a"));

        session.report(PlayerReport::EndOfTrack {
            track_id: Some("a".into()),
        });
        let next = cluster(&mut events).await.playback.unwrap();
        assert_eq!(next.track_id.as_deref(), Some("b"));
        assert_eq!(player.last_track().as_deref(), Some("b"));

        session.report(PlayerReport::EndOfTrack {
            track_id: Some("a".into()),
        });
        let late = cluster(&mut events).await.playback.unwrap();
        assert_eq!(
            late.track_id.as_deref(),
            Some("b"),
            "a late report for a doesn't skip b"
        );

        session.report(PlayerReport::Stopped {
            track_id: Some("a".into()),
        });
        cluster(&mut events).await;
        session.request(command(RemoteCommand::Pause));
        cluster(&mut events).await;
        session.request(command(RemoteCommand::Resume));
        cluster(&mut events).await;
        assert_eq!(
            player.commands.lock().unwrap().last(),
            Some(&RemoteCommand::Resume),
            "a's player stopping after b started leaves b playing"
        );

        session.request(command(RemoteCommand::AddToQueue {
            track_id: "q".into(),
        }));
        let queued = cluster(&mut events).await.playback.unwrap();
        assert!(queued.next[0].queued);

        session.request(Request::Volume {
            to: LOCAL_DEVICE_ID.to_string(),
            percent: 30,
        });
        let louder = cluster(&mut events).await;
        assert_eq!(louder.devices[0].volume_percent, Some(30));
        assert_eq!(*player.volume.lock().unwrap(), Some(30));
    }

    #[tokio::test]
    async fn stale_edits_and_other_devices_fail() {
        let player = Arc::new(FakePlayer::default());
        let (session, mut events) = LocalSession::start(api(), player, Load::background);
        cluster(&mut events).await;
        session.request(command(RemoteCommand::SetQueue {
            next: vec![QueueEntry::keyed("x", true, "q:9".into())],
            previous: vec![],
            revision: "old".into(),
        }));
        session.request(Request::Transfer {
            to: "phone".into(),
            playback: riff_api::remote::TransferPlayback::Keep,
        });
        let mut failed = 0;
        while failed < 2 {
            if let SessionEvent::RequestFailed(_) = events.recv().await.unwrap() {
                failed += 1;
            }
        }
    }
}
