use riff_config::setting::Values;
use riff_config::{Setting, SettingsSupport};

const MIN_CACHE_SIZE_MB: u32 = 100;
const MAX_CACHE_SIZE_MB: u32 = 16384;

pub(crate) fn settings_support() -> SettingsSupport {
    SettingsSupport::none()
        .with(&Setting::EQ_BANDS)
        .with(&[Setting::MonoAudio, Setting::Pan, Setting::PitchCents])
        .with_values(
            Setting::AudioCacheSizeMb,
            Values::Range {
                min: f64::from(MIN_CACHE_SIZE_MB),
                max: f64::from(MAX_CACHE_SIZE_MB),
            },
        )
}

pub(super) fn cache_size_mb() -> u32 {
    let stored = riff_config::SettingsStore::new().uint(Setting::AudioCacheSizeMb);
    settings_support().clamp(Setting::AudioCacheSizeMb, f64::from(stored)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_effects_and_the_cache_are_supported() {
        let support = settings_support();
        assert!(support.supports_any(&Setting::EQ_BANDS));
        assert!(support.supports(Setting::Pan));
        assert!(!support.supports(Setting::PlayerBitrate));
        assert!(!support.supports(Setting::AudioBackend));
        assert!(!support.supports(Setting::Normalisation));
        assert!(!support.supports(Setting::SkipExplicit));
        assert!(support.supports(Setting::ThemePreference));
    }

    #[test]
    fn the_cache_is_at_least_its_minimum() {
        let support = settings_support();
        assert_eq!(support.clamp(Setting::AudioCacheSizeMb, 64.0), 100.0);
        assert_eq!(support.clamp(Setting::AudioCacheSizeMb, 2048.0), 2048.0);
    }
}
