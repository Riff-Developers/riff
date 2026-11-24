use super::{SetResult, SettingsStore};
use crate::setting::Setting;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseWindowBehavior {
    #[default]
    Ask,
    MinimizeToBackground,
    StopAndQuit,
}

impl CloseWindowBehavior {
    const ALL: [Self; 3] = [Self::Ask, Self::MinimizeToBackground, Self::StopAndQuit];

    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::MinimizeToBackground => "minimize-to-background",
            Self::StopAndQuit => "stop-and-quit",
        }
    }

    fn from_str(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|behavior| behavior.as_str() == value)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowGeometry {
    pub width: i32,
    pub height: i32,
    pub is_maximized: bool,
}

impl SettingsStore {
    pub fn close_window_behavior(&self) -> CloseWindowBehavior {
        CloseWindowBehavior::from_str(&self.string(Setting::CloseWindowBehavior))
    }

    pub fn set_close_window_behavior(&self, behavior: CloseWindowBehavior) -> SetResult {
        self.set_string(Setting::CloseWindowBehavior, behavior.as_str())
    }

    pub fn window_geometry(&self) -> WindowGeometry {
        WindowGeometry {
            width: self.int(Setting::WindowWidth),
            height: self.int(Setting::WindowHeight),
            is_maximized: self.bool(Setting::WindowIsMaximized),
        }
    }

    pub fn set_window_geometry(&self, geometry: &WindowGeometry) -> SetResult {
        self.batch(|settings| {
            settings.set_int(Setting::WindowWidth, geometry.width)?;
            settings.set_int(Setting::WindowHeight, geometry.height)?;
            settings.set_bool(Setting::WindowIsMaximized, geometry.is_maximized)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../../../../../data/dev.diegovsky.Riff.gschema.xml");

    fn schema_nicks() -> Vec<&'static str> {
        let start = SCHEMA
            .find(r#"<enum id="dev.diegovsky.Riff.CloseWindowBehavior">"#)
            .expect("the schema has the enum");
        let end = start + SCHEMA[start..].find("</enum>").unwrap();
        SCHEMA[start..end]
            .split(r#"nick=""#)
            .skip(1)
            .map(|rest| &rest[..rest.find('"').unwrap()])
            .collect()
    }

    #[test]
    fn close_window_behaviors_match_the_schema() {
        let mut ours: Vec<_> = CloseWindowBehavior::ALL
            .map(CloseWindowBehavior::as_str)
            .into();
        let mut schema = schema_nicks();
        ours.sort();
        schema.sort();
        assert_eq!(ours, schema);
    }

    #[test]
    fn close_window_behavior_round_trips() {
        for behavior in CloseWindowBehavior::ALL {
            assert_eq!(CloseWindowBehavior::from_str(behavior.as_str()), behavior);
        }
        assert_eq!(
            CloseWindowBehavior::from_str("unknown"),
            CloseWindowBehavior::Ask
        );
    }
}
