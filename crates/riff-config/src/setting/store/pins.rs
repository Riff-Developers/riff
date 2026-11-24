use std::collections::HashMap;

use super::SettingsStore;
use crate::setting::Setting;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinnedKind {
    Playlist,
    Album,
    Artist,
    Track,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PinnedObject {
    pub id: String,
    pub kind: PinnedKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl PinnedObject {
    pub fn new(id: impl Into<String>, kind: PinnedKind) -> Self {
        Self {
            id: id.into(),
            kind,
            title: None,
        }
    }
}

type PinnedObjectsByUser = HashMap<String, Vec<serde_json::Value>>;

impl SettingsStore {
    pub fn pins(&self, user_id: &str) -> Vec<PinnedObject> {
        let Some(mut map) = self.pinned_map() else {
            return vec![];
        };
        map.remove(user_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect()
    }

    pub fn set_pins(&self, user_id: &str, objects: Vec<PinnedObject>) -> bool {
        let Some(mut map) = self.pinned_map() else {
            error!("Not saving pins over ones that couldn't be read");
            return false;
        };
        set_user_pins(&mut map, user_id, objects) && self.save_pinned_map(&map)
    }

    fn pinned_map(&self) -> Option<PinnedObjectsByUser> {
        let json = self.string(Setting::PinnedObjectsByUser);
        if json.is_empty() {
            return Some(HashMap::new());
        }
        serde_json::from_str(&json)
            .map_err(|e| error!("Failed to parse pinned objects: {e}"))
            .ok()
    }

    fn save_pinned_map(&self, map: &PinnedObjectsByUser) -> bool {
        match serde_json::to_string(map) {
            Ok(json) => self
                .set_string(Setting::PinnedObjectsByUser, &json)
                .map_err(|e| error!("Failed to save pinned objects: {e}"))
                .is_ok(),
            Err(e) => {
                error!("Failed to serialize pinned objects: {e}");
                false
            }
        }
    }
}

fn set_user_pins(map: &mut PinnedObjectsByUser, user_id: &str, objects: Vec<PinnedObject>) -> bool {
    let unread = map
        .get(user_id)
        .into_iter()
        .flatten()
        .filter(|entry| serde_json::from_value::<PinnedObject>((*entry).clone()).is_err())
        .cloned();
    let entries: Vec<serde_json::Value> = objects
        .iter()
        .filter_map(|object| serde_json::to_value(object).ok())
        .chain(unread)
        .collect();
    if map
        .get(user_id)
        .map_or(entries.is_empty(), |kept| *kept == entries)
    {
        return false;
    }
    if entries.is_empty() {
        map.remove(user_id);
    } else {
        map.insert(user_id.to_string(), entries);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(id: &str) -> PinnedObject {
        PinnedObject::new(id, PinnedKind::Playlist)
    }

    #[test]
    fn pins_are_kept_per_user() {
        let mut map = PinnedObjectsByUser::new();
        assert!(set_user_pins(&mut map, "user1", vec![pl("pl1")]));
        assert!(!set_user_pins(&mut map, "user1", vec![pl("pl1")]));
        assert!(!map.contains_key("user2"));
        assert!(!set_user_pins(&mut map, "user2", vec![]));
    }

    #[test]
    fn no_pins_leave_no_entry() {
        let mut map = PinnedObjectsByUser::new();
        set_user_pins(&mut map, "user1", vec![pl("pl1")]);
        assert!(set_user_pins(&mut map, "user1", vec![]));
        assert!(map.is_empty());
    }

    #[test]
    fn entries_it_cant_read_are_kept() {
        let mut map: PinnedObjectsByUser = serde_json::from_str(
            r#"{"user1":[{"id":"sh","kind":"show"}],"user2":[{"id":"x","kind":"future"}]}"#,
        )
        .unwrap();
        assert!(set_user_pins(&mut map, "user1", vec![pl("pl1")]));
        assert_eq!(map["user1"].len(), 2, "the show pin stays");
        assert_eq!(map["user2"].len(), 1, "another user's pins are left alone");
    }

    #[test]
    fn titles_round_trip() {
        let object = PinnedObject {
            title: Some("Title".into()),
            ..PinnedObject::new("al", PinnedKind::Album)
        };
        let json = serde_json::to_string(&vec![object.clone()]).unwrap();
        let back: Vec<PinnedObject> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, [object]);
    }
}
