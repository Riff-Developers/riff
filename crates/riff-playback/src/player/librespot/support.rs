use riff_config::setting::{Choice, Unavailable, Values};
use riff_config::{Setting, SettingsSupport};

const QUALITY: &[Choice] = &[
    Choice::new("96"),
    Choice::new("160"),
    Choice::new("320"),
    Choice::unavailable("lossless", Unavailable::LegalRestrictions),
];

pub(crate) fn settings_support() -> SettingsSupport {
    let others: Vec<Setting> = Setting::ALL
        .iter()
        .copied()
        .filter(|&setting| setting.depends_on_player() && setting != Setting::PlayerBitrate)
        .collect();
    SettingsSupport::none()
        .with(&others)
        .with_values(Setting::PlayerBitrate, Values::Choices(QUALITY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_every_player_setting() {
        let support = settings_support();
        for &setting in Setting::ALL {
            assert!(support.supports(setting), "{setting:?}");
        }
    }

    #[test]
    fn lossless_is_listed_but_cant_be_picked() {
        let support = settings_support();
        assert!(support.allows_choice(Setting::PlayerBitrate, "320"));
        assert!(!support.allows_choice(Setting::PlayerBitrate, "lossless"));
    }
}
