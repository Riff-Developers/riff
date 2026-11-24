use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use account::Account;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use riff_audio_engine::Effects;
use riff_auth::RiffOauthClient;
use tokio::task;
use url::Url;

use crate::{PlayerError, PlayerEvent};
use riff_auth::{SessionTokens, TokenStore};
use riff_core::playback::PlaybackAction;
use riff_core::session::SessionAction;
mod account;
mod error;
mod librespot;
pub mod soloist;
pub use error::SpotifyError;
pub use librespot::*;
use riff_api::remote::RemoteCommand;
use riff_api::ContentUris;

const DEVICE_NAME: &str = "Riff";

#[derive(Debug, Clone)]
pub enum Command {
    Restore,
    InitLogin,
    CompleteLogin,
    Logout,
    LoggedIn {
        user_id: String,
        session_token: Option<String>,
        explicit_filter_locked: bool,
    },
    ControlReceiver(RemoteCommand),
    SetReceiverVolume(u8),
    SetSkipExplicit(bool),
    ReloadSettings(SpotifyPlayerSettings),
    SetEqualizer {
        bands: [f64; 10],
    },
    SetMono {
        enabled: bool,
    },
    SetPan {
        pan: f64,
    },
    SetPitch {
        cents: f64,
    },
    ReconnectSession,
    RefreshToken,
    ReceiverGone {
        generation: u32,
    },
    TrackUnavailable,
    MarkPlaybackVerified,
    RecheckExplicitFilter,
    ExplicitFilterRechecked {
        filter_enabled: bool,
        filter_locked: bool,
    },
    #[cfg(debug_assertions)]
    DevKillPlayer,
    #[cfg(debug_assertions)]
    DevKillSession,
    #[cfg(debug_assertions)]
    DevExpireToken,
    #[cfg(debug_assertions)]
    DevResetDrmVerification,
    #[cfg(debug_assertions)]
    DevSimulateTrackUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Librespot,
    Soloist,
}

impl Engine {
    pub fn chosen() -> Self {
        let store = riff_config::SettingsStore::new();
        let saved = store.string(riff_config::Setting::PlaybackEngine);
        if saved == Self::Soloist.as_str() {
            Self::Soloist
        } else {
            Self::Librespot
        }
    }

    pub fn settings_support(self) -> riff_config::SettingsSupport {
        match self {
            Self::Librespot => librespot::settings_support(),
            Self::Soloist => soloist::settings_support(),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Librespot => "librespot",
            Self::Soloist => "soloist",
        }
    }

    pub fn choose(self) {
        let store = riff_config::SettingsStore::new();
        if let Err(e) = store.set_string(riff_config::Setting::PlaybackEngine, self.as_str()) {
            warn!("Couldn't save the playback engine: {e:?}");
        }
    }
}

#[derive(Clone)]
pub struct ThisComputer {
    librespot: ThisDevice,
    soloist: soloist::Soloist,
    uses_soloist: Arc<AtomicBool>,
}

impl ThisComputer {
    pub fn new(librespot: ThisDevice, soloist: soloist::Soloist) -> Self {
        let this = Self {
            librespot,
            soloist,
            uses_soloist: Arc::default(),
        };
        this.set_engine(Engine::chosen());
        this
    }

    pub fn soloist(&self) -> &soloist::Soloist {
        &self.soloist
    }

    pub fn set_engine(&self, engine: Engine) {
        self.uses_soloist
            .store(engine == Engine::Soloist, Ordering::Relaxed);
    }

    fn device(&self) -> &dyn riff_core::remote::LocalDevice {
        if self.uses_soloist.load(Ordering::Relaxed) {
            &self.soloist
        } else {
            &self.librespot
        }
    }
}

impl riff_core::remote::LocalDevice for ThisComputer {
    fn command(&self, command: RemoteCommand) {
        self.device().command(command)
    }

    fn set_volume(&self, percent: u8) {
        self.device().set_volume(percent)
    }

    fn plays(&self, command: &RemoteCommand) -> bool {
        self.device().plays(command)
    }

    fn listed_as(&self) -> Option<String> {
        self.device().listed_as()
    }

    fn take_playback(&self) -> bool {
        self.device().take_playback()
    }

    fn skips_explicit(&self) -> bool {
        self.device().skips_explicit()
    }
}

#[derive(Clone)]
pub struct ThisDevice {
    commands: UnboundedSender<Command>,
}

impl ThisDevice {
    pub fn new(commands: UnboundedSender<Command>) -> Self {
        Self { commands }
    }
}

impl riff_core::remote::LocalDevice for ThisDevice {
    fn command(&self, command: RemoteCommand) {
        let _ = self
            .commands
            .unbounded_send(Command::ControlReceiver(command));
    }

    fn set_volume(&self, percent: u8) {
        let _ = self
            .commands
            .unbounded_send(Command::SetReceiverVolume(percent));
    }

    fn skips_explicit(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub(crate) struct PlayerEvents {
    sender: UnboundedSender<PlayerEvent>,
}

impl PlayerEvents {
    fn new(sender: UnboundedSender<PlayerEvent>) -> Self {
        Self { sender }
    }

    fn send(&self, event: impl Into<PlayerEvent>) {
        self.sender.unbounded_send(event.into()).unwrap();
    }

    fn set_this_device_id(&self, id: Option<String>) {
        self.send(PlaybackAction::SetThisDeviceId(id))
    }

    fn report(&self, report: riff_core::remote::PlayerReport) {
        self.send(PlaybackAction::PlayerReport(report))
    }

    fn token_login_successful(&self, username: String) {
        self.send(SessionAction::LoginSucceeded(username))
    }

    fn refresh_successful(&self) {
        self.send(SessionAction::TokenRefreshed)
    }

    fn set_explicit_filter_locked(&self, locked: bool) {
        self.send(PlaybackAction::SetExplicitFilterLocked(locked))
    }

    fn set_skip_explicit(&self, skip: bool) {
        self.send(PlaybackAction::SetSkipExplicit(skip))
    }

    fn report_error(&self, error: SpotifyError) {
        match error {
            SpotifyError::NotPremium => self.send(SessionAction::NotPremium),
            SpotifyError::LoginFailed => self.send(SessionAction::LoginFailed),
            SpotifyError::LoggedOut => self.send(SessionAction::Logout),
            error => self.send(PlayerEvent::Error(PlayerError::Player(error))),
        }
    }

    fn login_challenge_started(&self, url: Url) {
        self.send(SessionAction::OpenLoginUrl(url))
    }

    fn set_connection_lost(&self, lost: bool) {
        self.send(SessionAction::SetConnectionLost(lost))
    }

    fn set_engine(&self, status: riff_core::events::EngineStatus) {
        self.send(SessionAction::SetEngine(status))
    }

    fn report_soloist_error(&self, error: soloist::SoloistError) {
        self.send(PlayerEvent::Error(PlayerError::Soloist(error)))
    }
}

#[tokio::main]
async fn player_main(
    player_settings: SpotifyPlayerSettings,
    events: UnboundedSender<PlayerEvent>,
    token_store: TokenStore,
    session_tokens: SessionTokens,
    sender: UnboundedSender<Command>,
    mut receiver: UnboundedReceiver<Command>,
    effects: Effects,
    uris: ContentUris,
) {
    let oauth_client = Arc::new(RiffOauthClient::new(token_store, session_tokens));
    let player_settings = Arc::new(std::sync::Mutex::new(player_settings));
    let librespot = {
        let (oauth_client, events) = (Arc::clone(&oauth_client), events.clone());
        let effects = effects.clone();
        let player_settings = Arc::clone(&player_settings);
        account::Librespot::new(move || {
            info!("Starting the librespot player");
            let (player, player_commands) = unbounded::<Command>();
            let settings = player_settings.lock().unwrap().clone();
            let librespot = SpotifyPlayer::new(
                settings,
                PlayerEvents::new(events.clone()),
                Arc::clone(&oauth_client),
                player.clone(),
                effects.clone(),
                uris,
            );
            task::spawn(async move {
                librespot.start(player_commands).await;
            });
            player
        })
    };
    let mut account = Account::new(oauth_client, sender, librespot, PlayerEvents::new(events));
    while let Some(command) = receiver.next().await {
        match command {
            Command::Restore
            | Command::InitLogin
            | Command::CompleteLogin
            | Command::Logout
            | Command::RefreshToken => account.handle(command).await,
            #[cfg(debug_assertions)]
            Command::DevExpireToken => account.handle(command).await,
            Command::SetEqualizer { .. }
            | Command::SetMono { .. }
            | Command::SetPan { .. }
            | Command::SetPitch { .. } => apply_effect(&effects, &command),
            command => {
                if let Command::ReloadSettings(settings) = &command {
                    *player_settings.lock().unwrap() = settings.clone();
                }
                match account.librespot() {
                    Some(player) => {
                        let _ = player.unbounded_send(command);
                    }
                    None => debug!("The librespot player isn't running; dropped {command:?}"),
                }
            }
        }
    }
}

fn apply_effect(effects: &Effects, command: &Command) {
    match *command {
        Command::SetEqualizer { bands } => effects.eq.update(bands),
        Command::SetMono { enabled } => effects.mono.update(enabled),
        Command::SetPan { pan } => effects.pan.update(pan),
        Command::SetPitch { cents } => effects.pitch.update(cents),
        _ => {}
    }
}

pub fn start_player_service(
    player_settings: SpotifyPlayerSettings,
    events: UnboundedSender<PlayerEvent>,
    token_store: TokenStore,
    session_tokens: SessionTokens,
    effects: Effects,
    uris: ContentUris,
) -> UnboundedSender<Command> {
    let (sender, receiver) = unbounded::<Command>();
    let sender_clone = sender.clone();
    std::thread::spawn(move || {
        player_main(
            player_settings,
            events,
            token_store,
            session_tokens,
            sender_clone,
            receiver,
            effects,
            uris,
        )
    });
    sender
}
