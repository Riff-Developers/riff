use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use librespot::connect::{
    ConnectConfig, LoadContextOptions, LoadRequest, LoadRequestOptions, Options, PlayingTrack,
    Spirc,
};
use librespot::core::authentication::Credentials;
use librespot::core::config::DeviceType;
use librespot::core::dealer::protocol::TransferOptions;
use librespot::core::session::Session;
use librespot::core::spclient::TransferRequest;
use librespot::core::SpotifyUri;
use librespot::playback::mixer::Mixer;
use librespot::playback::player::{Player, PlayerEvent};
use riff_api::models::RepeatMode;
use riff_api::remote::RemoteCommand;
use riff_api::ContentUris;
use tokio::task::JoinHandle;

const FILTER_EXPLICIT: &str = "filter-explicit-content";
const RESYNC_TOLERANCE_MS: u32 = 1_500;
const RESYNC_WINDOW: Duration = Duration::from_secs(30);
const TAKE_BACK_WAIT: Duration = Duration::from_secs(3);
const TAKE_BACK_ATTEMPTS: u32 = 3;
const DEATH_CHECK: Duration = Duration::from_millis(50);
const SOCKET_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const LEAVE_TIMEOUT: Duration = Duration::from_secs(3);

async fn died(session: &Session) {
    while !session.is_invalid() {
        tokio::time::sleep(DEATH_CHECK).await;
    }
}

#[derive(Clone, Debug)]
struct LeftOff {
    track: SpotifyUri,
    position_ms: u32,
    playing: bool,
    at: Instant,
}

impl LeftOff {
    fn expected(&self) -> u32 {
        let since = if self.playing {
            self.at.elapsed().as_millis() as u32
        } else {
            0
        };
        self.position_ms.saturating_add(since)
    }
}

pub struct Receiver {
    spirc: Arc<Spirc>,
    session: Session,
    mixer: Arc<dyn Mixer>,
    active: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    left_off: Arc<Mutex<Option<LeftOff>>>,
    uris: ContentUris,
    run: JoinHandle<()>,
    watch: JoinHandle<()>,
}

impl std::fmt::Debug for Receiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Receiver")
            .field("device_id", &self.session.device_id())
            .finish()
    }
}

impl Receiver {
    pub async fn start(
        session: Session,
        credentials: Credentials,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
        uris: ContentUris,
        gone: impl FnOnce() + Send + 'static,
    ) -> Result<Self, librespot::core::Error> {
        let config = ConnectConfig {
            name: crate::player::DEVICE_NAME.to_string(),
            device_type: DeviceType::Computer,
            initial_volume: mixer.volume(),
            ..Default::default()
        };
        let mut player_events = player.get_player_event_channel();
        let (spirc, spirc_task) = Spirc::new(
            config,
            session.clone(),
            credentials,
            player,
            Arc::clone(&mixer),
        )
        .await?;
        info!("listed as device {}", session.device_id());

        let run = {
            let session = session.clone();
            tokio::task::spawn(async move {
                tokio::select! {
                    _ = spirc_task => {}
                    _ = died(&session) => {
                        warn!("Connection died; leaving playback as it is for the next one");
                        let _ = tokio::time::timeout(SOCKET_CLOSE_TIMEOUT, session.dealer().close()).await;
                    }
                }
                gone();
            })
        };
        let spirc = Arc::new(spirc);
        let active = Arc::new(AtomicBool::new(false));
        let playing = Arc::new(AtomicBool::new(false));
        let left_off = Arc::new(Mutex::new(None));
        let watch = {
            let (active, playing, session) = (active.clone(), playing.clone(), session.clone());
            let (spirc, left_off) = (Arc::clone(&spirc), Arc::clone(&left_off));
            tokio::task::spawn(async move {
                while let Some(event) = player_events.recv().await {
                    if let PlayerEvent::Playing {
                        track_id,
                        position_ms,
                        ..
                    }
                    | PlayerEvent::Paused {
                        track_id,
                        position_ms,
                        ..
                    } = &event
                    {
                        resync(&spirc, &left_off, track_id, *position_ms);
                    }
                    match activity(&event) {
                        Some(Activity::Active(now)) => {
                            if now || !session.is_invalid() {
                                active.store(now, Ordering::Relaxed);
                            }
                        }
                        Some(Activity::Playing(now)) => playing.store(now, Ordering::Relaxed),
                        None => {}
                    }
                }
            })
        };

        Ok(Self {
            spirc,
            session,
            mixer,
            active,
            playing,
            left_off,
            uris,
            run,
            watch,
        })
    }

    pub fn device_id(&self) -> String {
        self.session.device_id().to_string()
    }

    pub fn username(&self) -> String {
        self.session.username()
    }

    pub fn is_alive(&self) -> bool {
        !self.session.is_invalid() && !self.run.is_finished()
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn take_back(&self, playing: bool, left_off: Option<(SpotifyUri, u32)>) {
        *self.left_off.lock().unwrap() = left_off.map(|(track, position_ms)| LeftOff {
            track,
            position_ms,
            playing,
            at: Instant::now(),
        });
        let request = move || TransferRequest {
            transfer_options: TransferOptions {
                restore_paused: Some(if playing { "resume" } else { "pause" }.to_string()),
                ..Default::default()
            },
        };
        if let Err(e) = self.spirc.transfer(Some(request())) {
            warn!("Couldn't take playback back: {e}");
        }
        let (spirc, session, active) = (
            Arc::clone(&self.spirc),
            self.session.clone(),
            Arc::clone(&self.active),
        );
        tokio::task::spawn(async move {
            for attempt in 1..=TAKE_BACK_ATTEMPTS {
                tokio::time::sleep(TAKE_BACK_WAIT).await;
                if active.load(Ordering::Relaxed) || session.is_invalid() {
                    return;
                }
                warn!("Playback didn't come back yet; asking again ({attempt})");
                if let Err(e) = session.spclient().put_connect_state_inactive(false).await {
                    warn!("Couldn't mark this device inactive: {e}");
                }
                if let Err(e) = spirc.transfer(Some(request())) {
                    warn!("Couldn't take playback back: {e}");
                }
            }
        });
    }

    pub fn set_skip_explicit(&self, skip: bool) {
        self.session
            .set_user_attribute(FILTER_EXPLICIT, if skip { "1" } else { "0" });
    }

    pub fn control(&self, command: RemoteCommand) {
        let options =
            |track: Option<PlayingTrack>, position_ms, paused: bool, shuffle: Option<bool>| {
                LoadRequestOptions {
                    start_playing: !paused,
                    seek_to: position_ms,
                    playing_track: track,
                    context_options: shuffle.map(|shuffle| {
                        LoadContextOptions::Options(Options {
                            shuffle,
                            ..Default::default()
                        })
                    }),
                }
            };
        let spirc = &self.spirc;
        let active = self.active.load(Ordering::Relaxed);
        let result = match command {
            RemoteCommand::Resume if !active => spirc.transfer(None),
            RemoteCommand::Resume => spirc.play(),
            RemoteCommand::Pause => spirc.pause(),
            RemoteCommand::SkipNext => spirc.next(),
            RemoteCommand::SkipPrevious => spirc.prev(),
            RemoteCommand::SeekTo(position_ms) => spirc.set_position_ms(position_ms),
            RemoteCommand::SetShuffle(shuffle) => spirc.shuffle(shuffle),
            RemoteCommand::SetRepeat(mode) => spirc
                .repeat(mode == RepeatMode::Context)
                .and_then(|_| spirc.repeat_track(mode == RepeatMode::Track)),
            RemoteCommand::PlayContext {
                context,
                track_id,
                track_index,
                position_ms,
                paused,
                shuffle,
            } => {
                let Some(context_uri) = self.uris.context(&context) else {
                    warn!("Receiver can't play {context:?}: the provider has no name for it");
                    return;
                };
                let track = match (track_id, track_index) {
                    (Some(id), _) => self.uris.track(&id).map(PlayingTrack::Uri),
                    (None, Some(index)) => Some(PlayingTrack::Index(index as u32)),
                    (None, None) => None,
                };
                self.activate().and_then(|_| {
                    spirc.load(LoadRequest::from_context_uri(
                        context_uri,
                        options(track, position_ms, paused, shuffle),
                    ))
                })
            }
            RemoteCommand::PlayTracks {
                track_ids,
                index,
                position_ms,
                paused,
                shuffle,
            } => {
                let uris = track_ids
                    .iter()
                    .filter_map(|id| self.uris.track(id))
                    .collect();
                let track = index.map(|i| PlayingTrack::Index(i as u32));
                self.activate().and_then(|_| {
                    spirc.load(LoadRequest::from_tracks(
                        uris,
                        options(track, position_ms, paused, shuffle),
                    ))
                })
            }
            RemoteCommand::AddToQueue { .. }
            | RemoteCommand::SetQueue { .. }
            | RemoteCommand::SkipTo(_) => {
                warn!("Receiver can't edit its queue directly");
                Ok(())
            }
        };
        if let Err(e) = result {
            warn!("Receiver didn't take the command: {e}");
        }
    }

    fn activate(&self) -> Result<(), librespot::core::Error> {
        if self.active.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.spirc.activate()
    }

    pub fn set_volume(&self, percent: u8) {
        let volume = (percent.min(100) as u32 * u16::MAX as u32 / 100) as u16;
        if self.active.load(Ordering::Relaxed) {
            if let Err(e) = self.spirc.set_volume(volume) {
                warn!("Receiver didn't take the volume: {e}");
            }
        }
        self.mixer.set_volume(volume);
    }

    #[cfg(debug_assertions)]
    pub fn kill(&self) {
        self.session.shutdown();
    }

    pub fn shutdown(self) {
        self.watch.abort();
        if !self.is_alive() {
            return;
        }
        let _ = self.spirc.shutdown();
        let (session, run) = (self.session, self.run);
        tokio::task::spawn(async move {
            if tokio::time::timeout(LEAVE_TIMEOUT, run).await.is_err() {
                debug!("Connection took too long to leave; closing it");
            }
            session.shutdown();
        });
    }
}

fn resync(spirc: &Spirc, left_off: &Mutex<Option<LeftOff>>, track: &SpotifyUri, position_ms: u32) {
    let Some(left_off) = left_off.lock().unwrap().take() else {
        return;
    };
    if left_off.at.elapsed() > RESYNC_WINDOW || left_off.track != *track {
        return;
    }
    let expected = left_off.expected();
    if expected.abs_diff(position_ms) <= RESYNC_TOLERANCE_MS {
        return;
    }
    info!("Playback came back at {position_ms}ms; moving it to {expected}ms, where it left off");
    if let Err(e) = spirc.set_position_ms(expected) {
        warn!("Couldn't move playback to where it left off: {e}");
    }
}

#[derive(Debug, PartialEq)]
enum Activity {
    Active(bool),
    Playing(bool),
}

fn activity(event: &PlayerEvent) -> Option<Activity> {
    Some(match event {
        PlayerEvent::SessionConnected { .. } => Activity::Active(true),
        PlayerEvent::SessionDisconnected { .. } => Activity::Active(false),
        PlayerEvent::Playing { .. } => Activity::Playing(true),
        PlayerEvent::Paused { .. } | PlayerEvent::Stopped { .. } => Activity::Playing(false),
        _ => return None,
    })
}

pub fn stable_device_id(path: &Path) -> String {
    if let Some(id) = std::fs::read_to_string(path)
        .ok()
        .map(|id| id.trim().to_string())
        .filter(|id| is_device_id(id))
    {
        return id;
    }
    let bytes: [u8; 20] = rand::random();
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, &id) {
        warn!("Cannot save the receiver's device id: {e}");
    }
    id
}

fn is_device_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_kept() {
        let dir = std::env::temp_dir().join(format!("riff-receiver-{}", std::process::id()));
        let path = dir.join("receiver-device-id");
        let _ = std::fs::remove_file(&path);
        let first = stable_device_id(&path);
        assert!(is_device_id(&first));
        assert_eq!(stable_device_id(&path), first);
        std::fs::write(&path, "garbage").unwrap();
        let replaced = stable_device_id(&path);
        assert!(is_device_id(&replaced));
        assert_ne!(replaced, first);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tracks_activation() {
        let connected = PlayerEvent::SessionConnected {
            connection_id: String::new(),
            user_name: String::new(),
        };
        assert_eq!(activity(&connected), Some(Activity::Active(true)));
        let other = PlayerEvent::PlayRequestIdChanged { play_request_id: 0 };
        assert_eq!(activity(&other), None);
    }
}
