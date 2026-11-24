use std::ops::{BitOr, BitOrAssign};

macro_rules! settings {
    (@restart restart $($rest:ident)*) => {
        true
    };
    (@restart $other:ident $($rest:ident)*) => {
        settings!(@restart $($rest)*)
    };
    (@restart) => {
        false
    };
    (@player player $($rest:ident)*) => {
        true
    };
    (@player $other:ident $($rest:ident)*) => {
        settings!(@player $($rest)*)
    };
    (@player) => {
        false
    };
    ($($variant:ident = $key:literal $(($($flag:ident),*))?,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Setting {
            $($variant,)*
        }

        impl Setting {
            pub const ALL: &'static [Setting] = &[$(Setting::$variant,)*];

            pub const fn key(self) -> &'static str {
                match self {
                    $(Setting::$variant => $key,)*
                }
            }

            pub const fn requires_restart(self) -> bool {
                match self {
                    $(Setting::$variant => settings!(@restart $($($flag)*)?),)*
                }
            }

            pub const fn depends_on_player(self) -> bool {
                match self {
                    $(Setting::$variant => settings!(@player $($($flag)*)?),)*
                }
            }
        }
    };
}

settings! {
    ThemePreference = "theme-preference",
    PlaybackEngine = "playback-engine",
    WindowWidth = "window-width",
    WindowHeight = "window-height",
    WindowIsMaximized = "window-is-maximized",
    CloseWindowBehavior = "close-window-behavior",
    InhibitSuspend = "inhibit-suspend",

    PlayerBitrate = "player-bitrate" (restart, player),
    AudioBackend = "audio-backend" (restart, player),
    GaplessPlayback = "gapless-playback" (restart, player),
    SkipExplicit = "skip-explicit" (player),
    AlsaDevice = "alsa-device" (restart, player),
    ApPort = "ap-port" (restart, player),
    VolumeCurve = "volume-curve" (restart, player),
    AudioFormat = "audio-format" (restart, player),
    Volume = "volume",

    Normalisation = "normalisation" (restart, player),
    NormalisationType = "normalisation-type" (restart, player),
    NormalisationMethod = "normalisation-method" (restart, player),
    NormalisationPregainDb = "normalisation-pregain-db" (restart, player),
    NormalisationThresholdDbfs = "normalisation-threshold-dbfs" (restart, player),
    NormalisationAttackMs = "normalisation-attack-ms" (restart, player),
    NormalisationReleaseMs = "normalisation-release-ms" (restart, player),
    NormalisationKneeDb = "normalisation-knee-db" (restart, player),

    MonoAudio = "mono-audio" (player),
    Pan = "pan" (player),
    PitchCents = "pitch-cents" (player),
    EqBand0 = "eq-band-0" (player),
    EqBand1 = "eq-band-1" (player),
    EqBand2 = "eq-band-2" (player),
    EqBand3 = "eq-band-3" (player),
    EqBand4 = "eq-band-4" (player),
    EqBand5 = "eq-band-5" (player),
    EqBand6 = "eq-band-6" (player),
    EqBand7 = "eq-band-7" (player),
    EqBand8 = "eq-band-8" (player),
    EqBand9 = "eq-band-9" (player),

    FeatureNormalisation = "feature-normalisation",
    FeaturePinnedObjects = "feature-pinned-objects",

    QueueSidePanel = "queue-side-panel",
    CardLayout = "card-layout",
    CardSize = "card-size",
    SortAlbums = "sort-albums",
    SortPlaylists = "sort-playlists",
    SortArtists = "sort-artists",

    PinnedObjectsByUser = "pinned-objects-by-user",
    DrmVerifiedUser = "drm-verified-user",
    SoloistPaired = "soloist-paired",

    MemoryCacheSizeMb = "memory-cache-size-mb",
    DiskCacheSizeMb = "disk-cache-size-mb",
    AudioCacheSizeMb = "audio-cache-size-mb" (restart, player),
}

const _: () = assert!(Setting::ALL.len() <= u128::BITS as usize);

impl Setting {
    pub const EQ_BANDS: [Setting; 10] = [
        Setting::EqBand0,
        Setting::EqBand1,
        Setting::EqBand2,
        Setting::EqBand3,
        Setting::EqBand4,
        Setting::EqBand5,
        Setting::EqBand6,
        Setting::EqBand7,
        Setting::EqBand8,
        Setting::EqBand9,
    ];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|setting| setting.key() == key)
    }

    pub fn sort_order(page: &str) -> Option<Self> {
        match page {
            "albums" => Some(Setting::SortAlbums),
            "playlists" => Some(Setting::SortPlaylists),
            "artists" => Some(Setting::SortArtists),
            _ => None,
        }
    }

    pub fn restart_required() -> impl Iterator<Item = Setting> {
        Self::ALL
            .iter()
            .copied()
            .filter(|setting| setting.requires_restart())
    }

    const fn bit(self) -> u128 {
        1 << self as u32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingSet(u128);

impl SettingSet {
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, setting: Setting) -> bool {
        self.0 & setting.bit() != 0
    }

    pub fn contains_any(self, settings: &[Setting]) -> bool {
        settings.iter().any(|&setting| self.contains(setting))
    }

    pub fn insert(&mut self, setting: Setting) {
        self.0 |= setting.bit();
    }

    pub fn iter(self) -> impl Iterator<Item = Setting> {
        Setting::ALL
            .iter()
            .copied()
            .filter(move |&setting| self.contains(setting))
    }
}

impl From<Setting> for SettingSet {
    fn from(setting: Setting) -> Self {
        Self(setting.bit())
    }
}

impl FromIterator<Setting> for SettingSet {
    fn from_iter<I: IntoIterator<Item = Setting>>(settings: I) -> Self {
        let mut set = Self::default();
        settings.into_iter().for_each(|setting| set.insert(setting));
        set
    }
}

impl BitOr for SettingSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for SettingSet {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsChanged {
    pub changed: SettingSet,
}

impl SettingsChanged {
    pub const fn contains(&self, setting: Setting) -> bool {
        self.changed.contains(setting)
    }

    pub fn contains_any(&self, settings: &[Setting]) -> bool {
        self.changed.contains_any(settings)
    }
}

impl From<Setting> for SettingsChanged {
    fn from(setting: Setting) -> Self {
        Self {
            changed: setting.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../../../../data/dev.diegovsky.Riff.gschema.xml");

    fn schema_keys() -> Vec<&'static str> {
        SCHEMA
            .split("<key name=")
            .skip(1)
            .map(|rest| {
                let quote = rest.chars().next().unwrap();
                rest[1..].split(quote).next().unwrap()
            })
            .collect()
    }

    #[test]
    fn every_schema_key_is_a_setting_and_no_other() {
        let mut keys: Vec<&str> = Setting::ALL.iter().map(|setting| setting.key()).collect();
        let mut schema = schema_keys();
        keys.sort_unstable();
        schema.sort_unstable();
        assert_eq!(keys, schema);
    }

    #[test]
    fn keys_map_back_to_their_setting() {
        for &setting in Setting::ALL {
            assert_eq!(Setting::from_key(setting.key()), Some(setting));
        }
        assert_eq!(Setting::from_key("no-such-key"), None);
    }

    #[test]
    fn settings_are_numbered_in_order() {
        for (index, &setting) in Setting::ALL.iter().enumerate() {
            assert_eq!(setting as usize, index);
        }
    }

    #[test]
    fn flags_are_read_from_the_list() {
        assert!(Setting::PlayerBitrate.requires_restart());
        assert!(Setting::PlayerBitrate.depends_on_player());
        assert!(!Setting::SkipExplicit.requires_restart());
        assert!(Setting::SkipExplicit.depends_on_player());
        assert!(!Setting::ThemePreference.requires_restart());
        assert!(!Setting::ThemePreference.depends_on_player());
    }

    #[test]
    fn sort_orders_have_a_setting_per_library_page() {
        assert_eq!(Setting::sort_order("albums"), Some(Setting::SortAlbums));
        assert_eq!(Setting::sort_order("songs"), None);
    }

    #[test]
    fn sets_hold_what_was_put_in() {
        let mut set: SettingSet = [Setting::Pan, Setting::EqBand3].into_iter().collect();
        assert!(set.contains(Setting::Pan));
        assert!(set.contains_any(&Setting::EQ_BANDS));
        assert!(!set.contains(Setting::Volume));
        set |= Setting::AudioCacheSizeMb.into();
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [Setting::Pan, Setting::EqBand3, Setting::AudioCacheSizeMb]
        );
        assert!(SettingSet::default().is_empty());
    }
}
