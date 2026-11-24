use crate::app::{
    components::{CardLayout, CardSize, EventListener, SortOrder},
    state::{PlaybackChanged, PlaybackFields, ViewPreferencesChanged},
    AppEvent,
};
use libadwaita::ColorScheme;
use librespot::playback::config::{AudioFormat, Bitrate, NormalisationMethod, NormalisationType};
use riff_playback::player::{AudioBackend, SpotifyPlayerSettings, VolumeCurveType};

pub use riff_config::setting::{CloseWindowBehavior, WindowGeometry};
use riff_config::{Setting, SettingsStore};

#[derive(Debug, Clone)]
pub struct RiffSettings {
    pub theme_preference: ColorScheme,
    pub player_settings: SpotifyPlayerSettings,
    pub window: WindowGeometry,
}

impl RiffSettings {
    pub fn new_from_gsettings() -> Option<Self> {
        let settings = SettingsStore::new();
        let theme_preference = match settings.enum_(Setting::ThemePreference) {
            0 => Some(ColorScheme::ForceLight),
            1 => Some(ColorScheme::ForceDark),
            2 => Some(ColorScheme::Default),
            _ => None,
        }?;
        Some(Self {
            theme_preference,
            player_settings: player_settings_from(&settings)?,
            window: settings.window_geometry(),
        })
    }
}

/// The player settings saved in GSettings.
pub fn player_settings() -> Option<SpotifyPlayerSettings> {
    player_settings_from(&SettingsStore::new())
}

fn player_settings_from(settings: &SettingsStore) -> Option<SpotifyPlayerSettings> {
    let bitrate = match settings.enum_(Setting::PlayerBitrate) {
        0 => Some(Bitrate::Bitrate96),
        1 => Some(Bitrate::Bitrate160),
        2 => Some(Bitrate::Bitrate320),
        _ => None,
    }?;
    let backend = match settings.enum_(Setting::AudioBackend) {
        0 => Some(AudioBackend::PulseAudio),
        1 => Some(AudioBackend::Alsa(settings.string(Setting::AlsaDevice))),
        _ => None,
    }?;
    let gapless = settings.bool(Setting::GaplessPlayback);

    let ap_port_val = settings.uint(Setting::ApPort);
    let ap_port = match ap_port_val {
        1..=65535 => Some(ap_port_val as u16),
        _ => None,
    };

    let volume = settings.double(Setting::Volume);
    let skip_explicit = settings.bool(Setting::SkipExplicit);

    let volume_curve = match settings.enum_(Setting::VolumeCurve) {
        0 => VolumeCurveType::Log,
        1 => VolumeCurveType::Linear,
        2 => VolumeCurveType::Cubic,
        _ => VolumeCurveType::Log,
    };

    let normalisation = settings.bool(Setting::Normalisation);
    let normalisation_type = match settings.enum_(Setting::NormalisationType) {
        0 => NormalisationType::Auto,
        1 => NormalisationType::Track,
        2 => NormalisationType::Album,
        _ => NormalisationType::Auto,
    };
    let normalisation_method = match settings.enum_(Setting::NormalisationMethod) {
        0 => NormalisationMethod::Dynamic,
        1 => NormalisationMethod::Basic,
        _ => NormalisationMethod::Dynamic,
    };
    let normalisation_pregain_db = settings.double(Setting::NormalisationPregainDb);
    let normalisation_threshold_dbfs = settings.double(Setting::NormalisationThresholdDbfs);
    let normalisation_attack_ms = settings.double(Setting::NormalisationAttackMs);
    let normalisation_release_ms = settings.double(Setting::NormalisationReleaseMs);
    let normalisation_knee_db = settings.double(Setting::NormalisationKneeDb);

    let audio_format = match settings.enum_(Setting::AudioFormat) {
        0 => AudioFormat::S16,
        1 => AudioFormat::S24,
        2 => AudioFormat::S24_3,
        3 => AudioFormat::S32,
        4 => AudioFormat::F32,
        5 => AudioFormat::F64,
        _ => AudioFormat::S16,
    };

    let eq_bands = Setting::EQ_BANDS.map(|band| settings.double(band));

    let mono_audio = settings.bool(Setting::MonoAudio);

    let pan = settings.double(Setting::Pan);

    let pitch_cents = settings.double(Setting::PitchCents);

    Some(SpotifyPlayerSettings {
        volume,

        skip_explicit,

        bitrate,
        backend,
        gapless,
        ap_port,

        volume_curve,

        normalisation,
        normalisation_type,
        normalisation_method,
        normalisation_pregain_db,
        normalisation_threshold_dbfs,
        normalisation_attack_ms,
        normalisation_release_ms,
        normalisation_knee_db,

        audio_format,

        mono_audio,

        pan,

        pitch_cents,

        eq_bands,
    })
}

impl Default for RiffSettings {
    fn default() -> Self {
        Self {
            theme_preference: ColorScheme::PreferDark,
            player_settings: Default::default(),
            window: Default::default(),
        }
    }
}

pub struct StateTracker {
    settings: SettingsStore,
}

type GResult = Result<(), glib::error::BoolError>;
impl StateTracker {
    pub fn new_from_gsettings() -> Self {
        Self {
            settings: SettingsStore::new(),
        }
    }
    fn on_playback_event(&self, event: &PlaybackChanged) -> GResult {
        let state = &event.state;
        if event.changed.contains(PlaybackFields::VOLUME) && state.local {
            self.settings.set_double(Setting::Volume, state.volume)?;
        }
        Ok(())
    }

    fn handle_event(&self, event: &AppEvent) -> GResult {
        if let Some(playback) = event.playback() {
            self.on_playback_event(playback)?;
        }
        match event {
            AppEvent::ViewPreferences(ViewPreferencesChanged::CardLayout(layout)) => {
                self.save_card_layout(*layout);
            }
            AppEvent::ViewPreferences(ViewPreferencesChanged::CardSize(size)) => {
                self.save_card_size(*size);
            }
            AppEvent::ViewPreferences(ViewPreferencesChanged::SortOrder(page, order)) => {
                self.save_sort_order(page, *order);
            }
            _ => (),
        }
        Ok(())
    }

    pub fn save_card_layout(&self, layout: CardLayout) {
        self.settings.set_card_layout(match layout {
            CardLayout::Vertical => "vertical",
            CardLayout::ImageOnly => "image-only",
            CardLayout::Horizontal => "horizontal",
        });
    }

    pub fn save_card_size(&self, size: CardSize) {
        self.settings.set_card_size(match size {
            CardSize::Small => "small",
            CardSize::Medium => "medium",
            CardSize::Large => "large",
        });
    }

    pub fn load_card_layout(&self) -> CardLayout {
        match self.settings.card_layout().as_str() {
            "image-only" => CardLayout::ImageOnly,
            "horizontal" => CardLayout::Horizontal,
            _ => CardLayout::Vertical,
        }
    }

    pub fn load_card_size(&self) -> CardSize {
        match self.settings.card_size().as_str() {
            "small" => CardSize::Small,
            "medium" => CardSize::Medium,
            _ => CardSize::Large,
        }
    }

    pub fn save_sort_order(&self, page: &str, order: SortOrder) {
        self.settings.set_sort_order(page, order.to_str());
    }

    pub fn load_sort_order(&self, page: &str) -> SortOrder {
        self.settings
            .sort_order(page)
            .map_or(SortOrder::RecentlyAdded, |key| SortOrder::parse_key(&key))
    }
}

impl EventListener for StateTracker {
    fn on_event(&mut self, event: &AppEvent) {
        if let Err(e) = self.handle_event(event) {
            error!("Trying to update gsettings: {e}");
        }
    }
}
