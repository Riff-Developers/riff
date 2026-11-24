use std::collections::HashSet;

use bitflags::bitflags;
use riff_api::models::Track;

#[derive(Clone, Debug)]
pub enum SelectionAction {
    Select(Vec<Track>),
    SelectKeyed(Vec<(String, Track)>),
    Deselect(Vec<String>),
    DeselectAll,
    Clear,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct SelectionFields: u8 {
        const MODE = 1 << 0;
        const ITEMS = 1 << 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionChanged {
    pub changed: SelectionFields,
    pub active: bool,
}

impl SelectionChanged {
    pub fn mode(active: bool) -> Self {
        Self {
            changed: SelectionFields::MODE,
            active,
        }
    }

    pub fn items(active: bool) -> Self {
        Self {
            changed: SelectionFields::ITEMS,
            active,
        }
    }

    pub fn mode_changed(&self) -> bool {
        self.changed.contains(SelectionFields::MODE)
    }
}

#[derive(Debug, Clone, Default)]
pub enum SelectionContext {
    Queue,
    Playlist,
    EditablePlaylist(String),
    SavedTracks,
    #[default]
    Default,
}

impl SelectionContext {
    pub fn is_queue(&self) -> bool {
        matches!(self, Self::Queue)
    }
}

#[derive(Default)]
pub struct SelectionState {
    selected_songs: Vec<(String, Track)>,
    selected_keys: HashSet<String>,
    selection_active: bool,
    pub context: SelectionContext,
}

impl SelectionState {
    fn select(&mut self, key: String, song: Track) -> bool {
        let selected = self.selected_keys.contains(&key);
        if !selected {
            self.selected_keys.insert(key.clone());
            self.selected_songs.push((key, song));
        }
        !selected
    }

    fn deselect(&mut self, key: &str) -> bool {
        self.selected_songs.retain(|(k, _)| k != key);
        self.selected_keys.remove(key)
    }

    pub fn is_selecting_in(&self, queue: bool) -> bool {
        self.selection_active && self.context.is_queue() == queue
    }

    pub fn set_mode(&mut self, context: Option<SelectionContext>) -> Option<bool> {
        let currently_active = self.selection_active;
        match (currently_active, context) {
            (true, Some(context)) if context.is_queue() == self.context.is_queue() => None,
            (_, Some(context)) => {
                *self = Default::default();
                self.selection_active = true;
                self.context = context;
                Some(true)
            }
            (true, None) => {
                *self = Default::default();
                Some(false)
            }
            _ => None,
        }
    }

    pub fn is_selection_enabled(&self) -> bool {
        self.selection_active
    }

    pub fn is_song_selected(&self, key: &str) -> bool {
        self.selected_keys.contains(key)
    }

    pub fn count(&self) -> usize {
        self.selected_keys.len()
    }

    // Clears (!) the selection, returns associated memory
    pub fn take_selection(&mut self) -> Vec<Track> {
        self.take_keyed_selection()
            .into_iter()
            .map(|(_, track)| track)
            .collect()
    }

    pub fn retain(&mut self, keep: impl Fn(&str, &Track) -> bool) -> Option<SelectionChanged> {
        let before = self.selected_songs.len();
        self.selected_songs.retain(|(key, track)| keep(key, track));
        if self.selected_songs.len() == before {
            return None;
        }
        self.selected_keys = self
            .selected_songs
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        Some(SelectionChanged::items(self.selection_active))
    }

    pub fn take_keyed_selection(&mut self) -> Vec<(String, Track)> {
        std::mem::take(self).selected_songs
    }

    fn select_rows(
        &mut self,
        rows: impl IntoIterator<Item = (String, Track)>,
    ) -> Vec<SelectionChanged> {
        let mut changed = false;
        for (key, track) in rows {
            changed |= self.select(key, track);
        }
        self.items_changed(changed)
    }

    fn items_changed(&self, changed: bool) -> Vec<SelectionChanged> {
        if changed {
            vec![SelectionChanged::items(self.selection_active)]
        } else {
            vec![]
        }
    }

    // Just have a look at the selection without changing it
    pub fn peek_selection(&self) -> impl Iterator<Item = &'_ Track> {
        self.selected_songs.iter().map(|(_, track)| track)
    }
}

impl SelectionState {
    pub fn apply(&mut self, action: SelectionAction) -> Vec<SelectionChanged> {
        match action {
            SelectionAction::Select(tracks) => {
                self.select_rows(tracks.into_iter().map(|t| (t.rri.id.clone(), t)))
            }
            SelectionAction::SelectKeyed(rows) => self.select_rows(rows),
            SelectionAction::Deselect(ids) => {
                let mut changed = false;
                for id in &ids {
                    changed |= self.deselect(id);
                }
                self.items_changed(changed)
            }
            SelectionAction::DeselectAll => {
                if self.selected_keys.is_empty() {
                    return vec![];
                }
                self.selected_songs.clear();
                self.selected_keys.clear();
                self.items_changed(true)
            }
            SelectionAction::Clear => {
                self.take_selection();
                vec![SelectionChanged::mode(false)]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::track as make_track;

    #[test]
    fn test_deselect_all_keeps_selection_mode() {
        let mut state = SelectionState::default();
        state.set_mode(Some(SelectionContext::Default));
        state.apply(SelectionAction::Select(vec![
            make_track("a"),
            make_track("b"),
        ]));
        assert_eq!(state.count(), 2);

        let events = state.apply(SelectionAction::DeselectAll);
        assert_eq!(events, [SelectionChanged::items(true)]);
        assert_eq!(state.count(), 0);
        assert!(state.is_selection_enabled());
    }

    #[test]
    fn test_selecting_elsewhere_starts_over() {
        let mut state = SelectionState::default();
        assert!(!state.is_selecting_in(false) && !state.is_selecting_in(true));
        assert_eq!(state.set_mode(Some(SelectionContext::Playlist)), Some(true));
        state.apply(SelectionAction::Select(vec![make_track("a")]));
        assert!(state.is_selecting_in(false) && !state.is_selecting_in(true));

        assert_eq!(state.set_mode(Some(SelectionContext::SavedTracks)), None);
        assert!(matches!(state.context, SelectionContext::Playlist));
        assert_eq!(state.count(), 1);

        assert_eq!(state.set_mode(Some(SelectionContext::Queue)), Some(true));
        assert!(state.is_selecting_in(true) && !state.is_selecting_in(false));
        assert_eq!(state.count(), 0);
        assert_eq!(state.set_mode(None), Some(false));
        assert!(!state.is_selecting_in(true));
    }

    #[test]
    fn test_selection_by_id_and_by_key() {
        let mut state = SelectionState::default();
        state.set_mode(Some(SelectionContext::Default));
        let events = state.apply(SelectionAction::Select(vec![
            make_track("a"),
            make_track("b"),
            make_track("a"),
        ]));
        assert_eq!(events.len(), 1);
        assert_eq!(state.count(), 2);
        assert!(state.is_song_selected("a"));
        assert_eq!(state.take_selection().len(), 2);
        assert!(!state.is_selection_enabled());

        state.set_mode(Some(SelectionContext::Queue));
        state.apply(SelectionAction::SelectKeyed(vec![
            ("k1".to_string(), make_track("a")),
            ("k2".to_string(), make_track("a")),
        ]));
        assert_eq!(state.count(), 2);
        assert!(state.is_song_selected("k1"));
        assert!(!state.is_song_selected("a"));
        state.apply(SelectionAction::Deselect(vec!["k1".to_string()]));
        let taken = state.take_keyed_selection();
        assert_eq!(taken.len(), 1);
        assert_eq!(
            (taken[0].0.as_str(), taken[0].1.rri.id.as_str()),
            ("k2", "a")
        );
    }
}
