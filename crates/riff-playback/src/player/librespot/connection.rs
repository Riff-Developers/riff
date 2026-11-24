use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use librespot::core::authentication::Credentials;
use librespot::core::config::SessionConfig;
use librespot::core::error::ErrorKind;
use librespot::core::session::Session;

use super::cache::{cached_credentials, librespot_cache_root, open_librespot_cache};
use super::receiver::{stable_device_id, Receiver};
use super::SpotifyPlayer;
use crate::player::{Command, SpotifyError};

const KNOWN_AP_PORTS: [Option<u16>; 4] = [None, Some(80), Some(443), Some(4070)];

impl SpotifyPlayer {
    pub(super) async fn connect(&mut self, credentials: Credentials) -> Result<(), SpotifyError> {
        #[cfg(debug_assertions)]
        if riff_api::is_simulate_offline() {
            warn!("Blocking connection (simulate offline is active)");
            return Err(SpotifyError::TechnicalError);
        }

        let ports = match self.settings.ap_port {
            Some(port) => vec![Some(port)],
            None => KNOWN_AP_PORTS.to_vec(),
        };
        for ap_port in ports {
            match self.connect_on(credentials.clone(), ap_port).await {
                Err(SpotifyError::TechnicalError) => continue,
                result => return result,
            }
        }
        Err(SpotifyError::TechnicalError)
    }

    pub(super) async fn connect_on(
        &mut self,
        credentials: Credentials,
        ap_port: Option<u16>,
    ) -> Result<(), SpotifyError> {
        debug!("Connecting (ap_port={ap_port:?})");
        let session_config = SessionConfig {
            device_id: stable_device_id(&librespot_cache_root().join("receiver-device-id")),
            ap_port,
            ..Default::default()
        };
        let session = Session::new(session_config, open_librespot_cache());
        let (player, kept) = match self.player.as_ref().filter(|p| !p.is_invalid()) {
            Some(player) => (Arc::clone(player), true),
            None => (self.new_player(session.clone()), false),
        };
        let mixer = Arc::clone(self.mixer.as_ref().expect("the player makes the mixer"));

        self.receiver_generation = self.receiver_generation.wrapping_add(1);
        let generation = self.receiver_generation;
        let commands = self.command_sender.clone();
        let gone = move || {
            let _ = commands.unbounded_send(Command::ReceiverGone { generation });
        };
        match Receiver::start(
            session.clone(),
            credentials,
            Arc::clone(&player),
            mixer,
            self.uris,
            gone,
        )
        .await
        {
            Ok(receiver) => {
                if kept {
                    info!("Carrying on with the same player");
                    player.set_session(session);
                }
                info!("Connected (ap_port={ap_port:?})");
                receiver.set_skip_explicit(self.skip_explicit);
                self.delegate.set_this_device_id(Some(receiver.device_id()));
                self.receiver = Some(receiver);
                Ok(())
            }
            Err(err) => {
                error!("Connecting failed (ap_port={ap_port:?}): {err}");
                match err.kind {
                    ErrorKind::PermissionDenied | ErrorKind::Unauthenticated => {
                        Err(SpotifyError::LoginFailed)
                    }
                    _ => Err(SpotifyError::TechnicalError),
                }
            }
        }
    }

    pub(super) fn drop_receiver(&mut self) {
        self.receiver_generation = self.receiver_generation.wrapping_add(1);
        if let Some(receiver) = self.receiver.take() {
            match self.clock.playing() {
                Some(playing) => self.resume = Some(playing),
                None if receiver.is_active() => self.resume = Some(receiver.is_playing()),
                None => {}
            }
            receiver.shutdown();
        }
    }

    pub(super) async fn reconnect(&mut self) -> Result<(), SpotifyError> {
        self.drop_receiver();
        self.connect(cached_credentials()?).await?;
        if let (Some(playing), Some(receiver)) = (self.resume.take(), self.receiver.as_ref()) {
            let left_off = self.clock.now();
            info!(
                "Taking playback back after reconnecting (playing: {playing}, at: {:?})",
                left_off.as_ref().map(|(_, position_ms)| position_ms)
            );
            receiver.take_back(playing, left_off);
        }
        Ok(())
    }

    pub(super) fn needs_reconnect(&self) -> bool {
        self.logged_in && !self.receiver.as_ref().is_some_and(Receiver::is_alive)
    }

    pub(super) async fn try_background_reconnect(&mut self) -> Result<(), SpotifyError> {
        if !self.needs_reconnect() {
            return Ok(());
        }
        if let Some(at) = self.next_reconnect_at {
            if Instant::now() < at {
                return Ok(());
            }
        }
        info!("Connection died; reconnecting");
        self.connection_lost.store(true, Ordering::Relaxed);
        self.delegate.set_connection_lost(true);
        match self.reconnect().await {
            Ok(()) => {
                info!("Reconnected");
                self.reconnect_attempts = 0;
                self.next_reconnect_at = None;
                self.connection_lost.store(false, Ordering::Relaxed);
                self.delegate.set_connection_lost(false);
                self.delegate.refresh_successful();
                Ok(())
            }
            Err(e @ (SpotifyError::LoggedOut | SpotifyError::NotPremium)) => Err(e),
            Err(e) => {
                warn!("Reconnecting failed (will retry): {e}");
                self.schedule_reconnect_retry();
                Ok(())
            }
        }
    }

    pub(super) fn schedule_reconnect_retry(&mut self) {
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
        let exp = self.reconnect_attempts.saturating_sub(1).min(3);
        let delay = Duration::from_millis(250 * (1u64 << exp));
        self.next_reconnect_at = Some(Instant::now() + delay);
        warn!("Next reconnect attempt in {}ms", delay.as_millis());

        let sender = self.command_sender.clone();
        tokio::task::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = sender.unbounded_send(Command::ReconnectSession);
        });
    }

    pub(super) fn spawn_session_watchdog(&mut self) {
        if self.watchdog_spawned {
            return;
        }
        self.watchdog_spawned = true;
        let sender = self.command_sender.clone();
        tokio::task::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                interval.tick().await;
                if sender.unbounded_send(Command::ReconnectSession).is_err() {
                    break;
                }
            }
        });
    }
}
