use crate::setting::Setting;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeatureFlag {
    Normalisation,
    PinnedObjects,
}

impl FeatureFlag {
    pub const ALL: &[FeatureFlag] = &[FeatureFlag::Normalisation, FeatureFlag::PinnedObjects];

    pub fn setting(&self) -> Setting {
        match self {
            FeatureFlag::Normalisation => Setting::FeatureNormalisation,
            FeatureFlag::PinnedObjects => Setting::FeaturePinnedObjects,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            FeatureFlag::Normalisation => "Audio Normalisation",
            FeatureFlag::PinnedObjects => "Pinned Items",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            FeatureFlag::Normalisation => {
                "Show audio normalisation settings for fine-tuning loudness between tracks."
            }
            FeatureFlag::PinnedObjects => {
                "Enable pinning saved playlists, albums, artists, and tracks to the navigation panel."
            }
        }
    }
}

#[cfg(feature = "gio")]
pub fn is_enabled(flag: FeatureFlag) -> bool {
    thread_local! {
        static SETTINGS: crate::setting::SettingsStore = crate::setting::SettingsStore::new();
    }
    SETTINGS.with(|settings| settings.bool(flag.setting()))
}
