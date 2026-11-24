mod archive;
pub mod download;
mod install_manager;
mod process;
mod protocol;
mod recovery;
mod support;
pub(crate) use support::settings_support;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;
use riff_api::remote::RemoteCommand;
use riff_api::ContentUris;
use riff_core::schedule::RunLog;
use serde_json::Value;
use tokio::sync::mpsc;

use super::PlayerEvents;
use crate::PlayerEvent;

pub const KEY_PAGE: &str = "https://developer.spotify.com/dashboard/soloist";

pub fn looks_like_key(key: &str) -> bool {
    let key = key.trim();
    key.len() >= 24 && key.chars().all(|c| c.is_ascii_graphic())
}

#[derive(Clone, Debug)]
pub struct SoloistConfig {
    pub root: PathBuf,
    pub cache_dir: PathBuf,
    pub device_name: String,
    pub binary: Option<PathBuf>,
    pub run_log: RunLog,
}

impl SoloistConfig {
    pub fn new(root: PathBuf, cache_dir: PathBuf, run_log: RunLog) -> Self {
        Self {
            root,
            cache_dir,
            device_name: crate::player::DEVICE_NAME.to_string(),
            binary: std::env::var_os("RIFF_SOLOIST_BIN").map(PathBuf::from),
            run_log,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|now| now.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoloistError {
    ExpiresSoon { days: i64 },
}

impl std::error::Error for SoloistError {}

impl fmt::Display for SoloistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpiresSoon { days } => write!(
                f,
                "Soloist stops working in {days} days, and an update couldn't be downloaded."
            ),
        }
    }
}

pub(super) enum Input {
    Command(Value),
    LoggedIn(bool),
    SetKey(String),
    Prepare,
    CheckForUpdate,
    MakeOutOfDate,
    Kill,
}

#[derive(Clone)]
pub struct Soloist {
    inputs: mpsc::UnboundedSender<Input>,
    device_id: Arc<Mutex<Option<String>>>,
    signed_in: Arc<AtomicBool>,
    uris: ContentUris,
}

impl Soloist {
    pub fn start(
        config: SoloistConfig,
        uris: ContentUris,
        events: UnboundedSender<PlayerEvent>,
        effects: riff_audio_engine::Effects,
    ) -> Self {
        info!("Playing through Soloist: {config:?}");
        let (inputs, receiver) = mpsc::unbounded_channel();
        let device_id = Arc::new(Mutex::new(None));
        let found_id = Arc::clone(&device_id);
        let signed_in = Arc::new(AtomicBool::new(false));
        let reported_signed_in = Arc::clone(&signed_in);
        std::thread::Builder::new()
            .name("soloist".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("couldn't start Soloist's runtime");
                runtime.block_on(process::run(
                    config,
                    receiver,
                    found_id,
                    reported_signed_in,
                    effects,
                    PlayerEvents::new(events),
                ));
            })
            .expect("couldn't start Soloist's thread");
        Self {
            inputs,
            device_id,
            signed_in,
            uris,
        }
    }

    pub fn set_logged_in(&self, logged_in: bool) {
        let _ = self.inputs.send(Input::LoggedIn(logged_in));
    }

    pub fn set_key(&self, key: String) {
        let _ = self.inputs.send(Input::SetKey(key));
    }

    pub fn prepare(&self) {
        let _ = self.inputs.send(Input::Prepare);
    }

    pub fn check_for_update(&self) {
        let _ = self.inputs.send(Input::CheckForUpdate);
    }

    pub fn make_out_of_date(&self) {
        let _ = self.inputs.send(Input::MakeOutOfDate);
    }

    pub fn kill(&self) {
        let _ = self.inputs.send(Input::Kill);
    }

    fn send(&self, message: Value) {
        if self.inputs.send(Input::Command(message)).is_err() {
            warn!("Soloist isn't running; a command was dropped");
        }
    }
}

impl riff_core::remote::LocalDevice for Soloist {
    fn command(&self, command: RemoteCommand) {
        for message in protocol::commands(command, &self.uris) {
            self.send(message);
        }
    }

    fn set_volume(&self, percent: u8) {
        self.send(protocol::set_volume(percent));
    }

    fn plays(&self, command: &RemoteCommand) -> bool {
        protocol::plays(command, &self.uris)
    }

    fn listed_as(&self) -> Option<String> {
        self.device_id.lock().unwrap().clone()
    }

    fn take_playback(&self) -> bool {
        if !self.signed_in.load(Ordering::Relaxed) {
            return false;
        }
        self.send(protocol::activate());
        true
    }
}

#[cfg(test)]
mod key_tests {
    use super::looks_like_key;

    #[test]
    fn only_unbroken_keys_pass() {
        assert!(looks_like_key("  aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3z_A-  "));
        assert!(!looks_like_key("Spotify Soloist API Key"));
        assert!(!looks_like_key("short"));
        assert!(!looks_like_key("aB3dE5fG7hJ9kL1m\nN3pQ5rS7tU9vW1xY3z"));
    }
}
