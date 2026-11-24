use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::stream::StreamExt;
use librespot::core::authentication::Credentials;
use librespot::playback::mixer::Mixer;
use librespot::playback::player::Player;
use riff_api::remote::RemoteCommand;
use riff_api::ContentUris;
use riff_audio_engine::Effects;
use riff_auth::RiffOauthClient;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

mod audio;
mod cache;
mod clock;
mod connection;
mod receiver;
mod settings;
mod support;

use super::{Command, PlayerEvents, SpotifyError};
use audio::mixer_set_volume;
use cache::{cached_credentials, note_session_minted, session_cache_usable};
use clock::PositionClock;
use receiver::Receiver;

pub use cache::{clear_audio_cache, set_cache_root};
pub use settings::{AudioBackend, SpotifyPlayerSettings, VolumeCurveType};
pub(crate) use support::settings_support;

const CONSECUTIVE_UNAVAILABLE_STOP_THRESHOLD: u32 = 10;

pub struct SpotifyPlayer {
    settings: SpotifyPlayerSettings,
    uris: ContentUris,
    mixer: Option<Arc<dyn Mixer>>,

    receiver: Option<Receiver>,
    player: Option<Arc<Player>>,
    clock: PositionClock,
    receiver_generation: u32,
    resume: Option<bool>,

    effects: Effects,

    oauth_client: Arc<RiffOauthClient>,
    command_sender: UnboundedSender<Command>,

    logged_in: bool,

    reconnect_attempts: u32,
    next_reconnect_at: Option<Instant>,
    watchdog_spawned: bool,

    connection_lost: Arc<AtomicBool>,
    consecutive_load_failures: Arc<AtomicU32>,
    has_played_since_login: Arc<AtomicBool>,
    current_user_id: Option<String>,

    explicit_filter_locked: bool,
    skip_explicit: bool,

    delegate: PlayerEvents,
}

impl SpotifyPlayer {
    pub(crate) fn new(
        settings: SpotifyPlayerSettings,
        delegate: PlayerEvents,
        oauth_client: Arc<RiffOauthClient>,
        command_sender: UnboundedSender<Command>,
        effects: Effects,
        uris: ContentUris,
    ) -> Self {
        let skip_explicit = settings.skip_explicit;
        Self {
            settings,
            uris,
            mixer: None,
            receiver: None,
            player: None,
            clock: PositionClock::default(),
            receiver_generation: 0,
            resume: None,
            effects,
            oauth_client,
            command_sender,
            logged_in: false,

            reconnect_attempts: 0,
            next_reconnect_at: None,
            watchdog_spawned: false,

            connection_lost: Arc::new(AtomicBool::new(false)),
            consecutive_load_failures: Arc::new(AtomicU32::new(0)),
            has_played_since_login: Arc::new(AtomicBool::new(false)),
            current_user_id: None,

            explicit_filter_locked: false,
            skip_explicit,

            delegate,
        }
    }

    async fn handle_and_notify(&mut self, action: Command) {
        if let Err(e) = self.handle(action).await {
            self.delegate.report_error(e);
        }
    }

    async fn handle(&mut self, action: Command) -> Result<(), SpotifyError> {
        match action {
            Command::ControlReceiver(command) => {
                match self.receiver.as_ref() {
                    Some(receiver) => receiver.control(command),
                    None => warn!("Not connected; dropping {command:?}"),
                }
                Ok(())
            }
            Command::SetReceiverVolume(percent) => {
                match (&self.receiver, &self.mixer) {
                    (Some(receiver), _) => receiver.set_volume(percent),
                    (None, Some(mixer)) => mixer_set_volume(&**mixer, percent as f64 / 100.0),
                    (None, None) => {}
                }
                Ok(())
            }
            Command::SetSkipExplicit(skip) => {
                self.skip_explicit = skip;
                if let Some(receiver) = self.receiver.as_ref() {
                    receiver.set_skip_explicit(skip);
                }
                Ok(())
            }
            Command::ReconnectSession => self.try_background_reconnect().await,
            Command::ReceiverGone { generation } => {
                if generation == self.receiver_generation {
                    warn!("Connection ended; reconnecting");
                    self.drop_receiver();
                    self.connection_lost.store(true, Ordering::Relaxed);
                    self.delegate.set_connection_lost(true);
                    self.try_background_reconnect().await?;
                }
                Ok(())
            }
            Command::TrackUnavailable => {
                if self.receiver.as_ref().is_some_and(|r| !r.is_alive()) {
                    warn!("Track load failed because the connection died: reconnecting");
                    return self.try_background_reconnect().await;
                }
                self.handle_unavailable_track();
                Ok(())
            }
            #[cfg(debug_assertions)]
            Command::DevSimulateTrackUnavailable => {
                warn!("[dev] Simulating an unavailable track failure");
                self.handle_unavailable_track();
                Ok(())
            }
            Command::MarkPlaybackVerified => {
                if let Some(user_id) = self.current_user_id.as_deref() {
                    if !user_id.is_empty()
                        && riff_config::SettingsStore::new().drm_verified_user() != user_id
                    {
                        info!("Recording account as verified playable");
                        riff_config::SettingsStore::new().set_drm_verified_user(user_id);
                    }
                }
                Ok(())
            }
            Command::Logout => {
                self.logged_in = false;
                self.drop_receiver();
                self.delegate.set_this_device_id(None);
                self.player = None;
                self.clock.clear();
                self.resume = None;
                self.reconnect_attempts = 0;
                self.next_reconnect_at = None;
                self.connection_lost.store(false, Ordering::Relaxed);
                self.consecutive_load_failures.store(0, Ordering::Relaxed);
                self.has_played_since_login.store(false, Ordering::Relaxed);
                self.delegate.set_connection_lost(false);
                Ok(())
            }
            Command::Restore | Command::InitLogin | Command::CompleteLogin => {
                warn!("The player doesn't log in: {action:?}");
                Ok(())
            }
            Command::SetEqualizer { .. }
            | Command::SetMono { .. }
            | Command::SetPan { .. }
            | Command::SetPitch { .. } => Ok(()),
            #[cfg(debug_assertions)]
            Command::DevExpireToken => Ok(()),
            Command::RefreshToken => Ok(()),
            Command::LoggedIn {
                user_id,
                session_token,
                explicit_filter_locked,
            } => {
                self.log_in(user_id, session_token, explicit_filter_locked)
                    .await
            }
            Command::ReloadSettings(settings) => {
                self.settings = settings;
                if !self.logged_in {
                    return Ok(());
                }
                self.drop_receiver();
                self.player = None;
                self.mixer.take();
                self.reconnect().await
            }
            Command::RecheckExplicitFilter => {
                if self.explicit_filter_locked {
                    return Ok(());
                }

                let oauth_client = Arc::clone(&self.oauth_client);
                let command_sender = self.command_sender.clone();
                tokio::task::spawn(async move {
                    let token = match oauth_client.get_valid_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("Explicit filter re-check: could not get token: {e:?}");
                            return;
                        }
                    };
                    match riff_api::check_user_profile(&token.access_token).await {
                        Ok(profile) => {
                            let _ =
                                command_sender.unbounded_send(Command::ExplicitFilterRechecked {
                                    filter_enabled: profile.explicit_filter_enabled,
                                    filter_locked: profile.explicit_filter_locked,
                                });
                        }
                        Err(e) => {
                            warn!("Failed to re-check explicit content filter: {e:?}");
                        }
                    }
                });
                Ok(())
            }
            Command::ExplicitFilterRechecked {
                filter_enabled,
                filter_locked,
            } => {
                if filter_locked && !self.explicit_filter_locked {
                    info!("Explicit content filter is now locked; syncing Riff's filter");
                }
                self.explicit_filter_locked = filter_locked;
                self.delegate.set_explicit_filter_locked(filter_locked);
                if filter_enabled {
                    self.delegate.set_skip_explicit(true);
                }
                Ok(())
            }
            #[cfg(debug_assertions)]
            Command::DevKillPlayer | Command::DevKillSession => {
                warn!("[dev] Killing the connection; the watchdog rebuilds it");
                if let Some(receiver) = self.receiver.as_ref() {
                    receiver.kill();
                }
                Ok(())
            }
            #[cfg(debug_assertions)]
            Command::DevResetDrmVerification => {
                warn!("[dev] Clearing verified-playable marker and re-arming DRM detection");
                riff_config::SettingsStore::new().clear_drm_verified_user();
                self.has_played_since_login.store(false, Ordering::Relaxed);
                self.consecutive_load_failures.store(0, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    async fn log_in(
        &mut self,
        user_id: String,
        session_token: Option<String>,
        explicit_filter_locked: bool,
    ) -> Result<(), SpotifyError> {
        self.consecutive_load_failures.store(0, Ordering::Relaxed);
        let known_good =
            !user_id.is_empty() && riff_config::SettingsStore::new().drm_verified_user() == user_id;
        self.current_user_id = Some(user_id);
        self.has_played_since_login
            .store(known_good, Ordering::Relaxed);
        if known_good {
            debug!("Account previously verified as playable; DRM detection disabled");
        }
        self.explicit_filter_locked = explicit_filter_locked;

        match session_token {
            Some(token) => {
                self.connect(Credentials::with_access_token(&token)).await?;
                note_session_minted();
            }
            None => {
                if !session_cache_usable() {
                    info!("No usable session credentials on disk; showing login");
                    return Err(SpotifyError::LoggedOut);
                }
                info!(
                    "Reusing cached session credentials (ap_port: {:?})",
                    self.settings.ap_port
                );
                self.connect(cached_credentials()?).await?;
            }
        }
        self.finish_login();
        Ok(())
    }

    fn finish_login(&mut self) {
        let username = self
            .receiver
            .as_ref()
            .map(Receiver::username)
            .or_else(|| self.current_user_id.clone())
            .unwrap_or_default();
        info!("Connected as user: {username}");
        self.logged_in = true;
        self.reconnect_attempts = 0;
        self.next_reconnect_at = None;

        self.spawn_session_watchdog();
        self.delegate.token_login_successful(username);
    }

    fn handle_unavailable_track(&mut self) {
        warn!("Track unavailable; it's skipped");
        let failures = self
            .consecutive_load_failures
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if failures >= CONSECUTIVE_UNAVAILABLE_STOP_THRESHOLD {
            warn!("{failures} tracks failed to load back-to-back; stopping playback");
            if !self.has_played_since_login.load(Ordering::Relaxed) {
                self.delegate.report_error(SpotifyError::PlaybackDrmBlocked);
            } else {
                self.delegate
                    .report_error(SpotifyError::PlaybackTemporarilyUnavailable);
            }
            self.consecutive_load_failures.store(0, Ordering::Relaxed);
            if let Some(receiver) = self.receiver.as_ref() {
                receiver.control(RemoteCommand::Pause);
            }
            return;
        }
        // The account's explicit filter may have changed; re-query it.
        let _ = self
            .command_sender
            .unbounded_send(Command::RecheckExplicitFilter);
    }

    pub async fn start(mut self, mut receiver: UnboundedReceiver<Command>) {
        while let Some(action) = receiver.next().await {
            self.handle_and_notify(action).await;
        }
    }
}
