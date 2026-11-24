use super::{Setting, SettingSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    LegalRestrictions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: &'static str,
    pub unavailable: Option<Unavailable>,
}

impl Choice {
    pub const fn new(value: &'static str) -> Self {
        Self {
            value,
            unavailable: None,
        }
    }

    pub const fn unavailable(value: &'static str, why: Unavailable) -> Self {
        Self {
            value,
            unavailable: Some(why),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Values {
    Choices(&'static [Choice]),
    Range { min: f64, max: f64 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsSupport {
    settings: SettingSet,
    values: Vec<(Setting, Values)>,
}

impl SettingsSupport {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with(mut self, settings: &[Setting]) -> Self {
        for &setting in settings {
            self.settings.insert(setting);
        }
        self
    }

    pub fn with_values(mut self, setting: Setting, values: Values) -> Self {
        self = self.with(&[setting]);
        self.values.retain(|(other, _)| *other != setting);
        self.values.push((setting, values));
        self
    }

    pub fn supports(&self, setting: Setting) -> bool {
        !setting.depends_on_player() || self.settings.contains(setting)
    }

    pub fn supports_any(&self, settings: &[Setting]) -> bool {
        settings.iter().any(|&setting| self.supports(setting))
    }

    pub fn values(&self, setting: Setting) -> Option<Values> {
        if !self.supports(setting) {
            return None;
        }
        self.values
            .iter()
            .find(|(other, _)| *other == setting)
            .map(|&(_, values)| values)
    }

    pub fn choices(&self, setting: Setting) -> Option<&'static [Choice]> {
        match self.values(setting)? {
            Values::Choices(choices) => Some(choices),
            Values::Range { .. } => None,
        }
    }

    pub fn range(&self, setting: Setting) -> Option<(f64, f64)> {
        match self.values(setting)? {
            Values::Range { min, max } => Some((min, max)),
            Values::Choices(_) => None,
        }
    }

    pub fn allows_choice(&self, setting: Setting, value: &str) -> bool {
        if !self.supports(setting) {
            return false;
        }
        match self.choices(setting) {
            Some(choices) => choices
                .iter()
                .any(|choice| choice.value == value && choice.unavailable.is_none()),
            None => true,
        }
    }

    pub fn clamp(&self, setting: Setting, value: f64) -> f64 {
        match self.range(setting) {
            Some((min, max)) => value.clamp(min, max),
            None => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUALITY: &[Choice] = &[
        Choice::new("96"),
        Choice::new("160"),
        Choice::unavailable("lossless", Unavailable::LegalRestrictions),
    ];

    #[test]
    fn settings_that_dont_depend_on_the_player_are_always_supported() {
        let none = SettingsSupport::none();
        assert!(none.supports(Setting::ThemePreference));
        assert!(none.supports(Setting::MemoryCacheSizeMb));
        assert!(!none.supports(Setting::PlayerBitrate));
        assert!(!none.supports_any(&Setting::EQ_BANDS));
    }

    #[test]
    fn only_listed_player_settings_are_supported() {
        let support = SettingsSupport::none().with(&Setting::EQ_BANDS);
        assert!(support.supports(Setting::EqBand4));
        assert!(!support.supports(Setting::Pan));
        assert_eq!(support.values(Setting::EqBand4), None);
    }

    #[test]
    fn choices_narrow_what_can_be_picked() {
        let support = SettingsSupport::none()
            .with_values(Setting::PlayerBitrate, Values::Choices(QUALITY))
            .with(&[Setting::AudioFormat]);
        assert_eq!(support.choices(Setting::PlayerBitrate), Some(QUALITY));
        assert!(support.allows_choice(Setting::PlayerBitrate, "160"));
        assert!(!support.allows_choice(Setting::PlayerBitrate, "320"));
        assert!(!support.allows_choice(Setting::PlayerBitrate, "lossless"));
        assert!(support.allows_choice(Setting::AudioFormat, "f32"));
        assert!(!support.allows_choice(Setting::VolumeCurve, "log"));
        assert!(support.allows_choice(Setting::ThemePreference, "dark"));
    }

    #[test]
    fn ranges_clamp_values() {
        let support = SettingsSupport::none().with_values(
            Setting::AudioCacheSizeMb,
            Values::Range {
                min: 100.0,
                max: 1000.0,
            },
        );
        assert_eq!(
            support.range(Setting::AudioCacheSizeMb),
            Some((100.0, 1000.0))
        );
        assert_eq!(support.clamp(Setting::AudioCacheSizeMb, 64.0), 100.0);
        assert_eq!(support.clamp(Setting::AudioCacheSizeMb, 500.0), 500.0);
        assert_eq!(support.clamp(Setting::DiskCacheSizeMb, 5.0), 5.0);
        assert_eq!(support.choices(Setting::AudioCacheSizeMb), None);
    }

    #[test]
    fn values_given_again_replace_the_first() {
        let support = SettingsSupport::none()
            .with_values(Setting::PlayerBitrate, Values::Choices(QUALITY))
            .with_values(Setting::PlayerBitrate, Values::Choices(&QUALITY[..1]));
        assert_eq!(support.choices(Setting::PlayerBitrate), Some(&QUALITY[..1]));
    }
}
