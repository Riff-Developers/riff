use std::env;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use librespot::core::session::Session;
use librespot::playback::audio_backend;
use librespot::playback::audio_backend::Sink;
use librespot::playback::config::{PlayerConfig, VolumeCtrl};
use librespot::playback::mixer::softmixer::SoftMixer;
use librespot::playback::mixer::{Mixer, MixerConfig};
use librespot::playback::player::{Player, PlayerEvent, PlayerEventChannel};
use riff_audio_engine::CaptureSink;
use riff_core::remote::PlayerReport;

use super::clock::{PositionClock, POSITION_UPDATE_INTERVAL};
use super::settings::{AudioBackend, VolumeCurveType};
use super::SpotifyPlayer;
use crate::player::{Command, PlayerEvents};

impl SpotifyPlayer {
    pub(super) fn new_player(&mut self, session: Session) -> Arc<Player> {
        let player = self.create_player(session);
        tokio::task::spawn(player_setup_delegate(
            player.get_player_event_channel(),
            self.delegate.clone(),
            self.command_sender.clone(),
            Arc::clone(&self.connection_lost),
            Arc::clone(&self.consecutive_load_failures),
            Arc::clone(&self.has_played_since_login),
            self.clock.clone(),
        ));
        self.player = Some(Arc::clone(&player));
        player
    }

    pub(super) fn create_player(&mut self, session: Session) -> Arc<Player> {
        let backend = self.settings.backend.clone();
        let audio_format = self.settings.audio_format;

        // Convert attack/release from milliseconds to coefficients
        let normalisation_attack_cf = librespot::playback::player::duration_to_coefficient(
            std::time::Duration::from_secs_f64(self.settings.normalisation_attack_ms / 1000.0),
        );
        let normalisation_release_cf = librespot::playback::player::duration_to_coefficient(
            std::time::Duration::from_secs_f64(self.settings.normalisation_release_ms / 1000.0),
        );

        let player_config = PlayerConfig {
            gapless: self.settings.gapless,
            bitrate: self.settings.bitrate,
            normalisation: self.settings.normalisation,
            normalisation_type: self.settings.normalisation_type,
            normalisation_method: self.settings.normalisation_method,
            normalisation_pregain_db: self.settings.normalisation_pregain_db,
            normalisation_threshold_dbfs: self.settings.normalisation_threshold_dbfs,
            normalisation_attack_cf,
            normalisation_release_cf,
            normalisation_knee_db: self.settings.normalisation_knee_db,
            position_update_interval: Some(POSITION_UPDATE_INTERVAL),
            ..Default::default()
        };
        info!("bitrate: {:?}", player_config.bitrate);
        info!(
            "volume curve: {:?}, dB range: {:.1}",
            self.settings.volume_curve,
            VolumeCtrl::DEFAULT_DB_RANGE
        );
        if player_config.normalisation {
            info!(
                "normalisation: type={:?}, method={:?}, pregain={:.1}dB",
                player_config.normalisation_type,
                player_config.normalisation_method,
                player_config.normalisation_pregain_db
            );
        }

        let volume = self.settings.volume;
        let volume_curve = self.settings.volume_curve;
        let soft_volume = self
            .mixer
            .get_or_insert_with(|| {
                let volume_ctrl = match volume_curve {
                    VolumeCurveType::Log => VolumeCtrl::Log(VolumeCtrl::DEFAULT_DB_RANGE),
                    VolumeCurveType::Linear => VolumeCtrl::Linear,
                    VolumeCurveType::Cubic => VolumeCtrl::Cubic(VolumeCtrl::DEFAULT_DB_RANGE),
                };
                let mix: Arc<dyn Mixer> = Arc::new(
                    SoftMixer::open(MixerConfig {
                        volume_ctrl,
                        ..Default::default()
                    })
                    .expect("Failed to create soft mixer"),
                );
                mixer_set_volume(&*mix, volume);
                mix
            })
            .get_soft_volume();

        let chain = self.effects.chain();

        Player::new(player_config, session, soft_volume, move || {
            let sink: Box<dyn Sink> = match backend {
                AudioBackend::PulseAudio => {
                    info!("using pulseaudio");
                    env::set_var("PULSE_PROP_application.name", "Riff");
                    let backend = audio_backend::find(Some("pulseaudio".to_string())).unwrap();
                    backend(None, audio_format)
                }
                AudioBackend::Alsa(device) => {
                    info!("using alsa ({device})");
                    let backend = audio_backend::find(Some("alsa".to_string())).unwrap();
                    backend(Some(device), audio_format)
                }
            };

            CaptureSink::wrap(sink, chain)
        })
    }
}

async fn player_setup_delegate(
    mut channel: PlayerEventChannel,
    delegate: PlayerEvents,
    command_sender: UnboundedSender<Command>,
    connection_lost: Arc<AtomicBool>,
    consecutive_load_failures: Arc<AtomicU32>,
    has_played_since_login: Arc<AtomicBool>,
    clock: PositionClock,
) {
    while let Some(event) = channel.recv().await {
        clock.follow(&event);
        match event {
            PlayerEvent::Unavailable { track_id, .. } => {
                warn!("Track could not be loaded: {track_id:?}");
                let _ = command_sender.unbounded_send(Command::TrackUnavailable);
                delegate.report(PlayerReport::Unavailable {
                    track_id: track_id.to_id().ok(),
                });
            }
            PlayerEvent::Paused { position_ms, .. } => {
                delegate.report(PlayerReport::Paused { position_ms });
            }
            PlayerEvent::EndOfTrack { track_id, .. } => delegate.report(PlayerReport::EndOfTrack {
                track_id: track_id.to_id().ok(),
            }),
            PlayerEvent::Stopped { track_id, .. } => delegate.report(PlayerReport::Stopped {
                track_id: track_id.to_id().ok(),
            }),
            PlayerEvent::Playing { position_ms, .. } => {
                delegate.report(PlayerReport::Playing { position_ms });
                if connection_lost.swap(false, Ordering::Relaxed) {
                    delegate.set_connection_lost(false);
                }
                // A track played: not DRM-blocked. Clear the failure run and,
                // on the first play this login, persist the account as known-good.
                consecutive_load_failures.store(0, Ordering::Relaxed);
                if !has_played_since_login.swap(true, Ordering::Relaxed) {
                    let _ = command_sender.unbounded_send(Command::MarkPlaybackVerified);
                }
            }
            _ => {}
        }
    }
    debug!("Player event channel closed (player was replaced or dropped)");
}

pub(super) fn mixer_set_volume(mixer: &dyn Mixer, volume: f64) {
    mixer.set_volume((VolumeCtrl::MAX_VOLUME as f64 * volume) as u16);
}
