use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use riff_api::models::Track;

use crate::events::{Event, ItemId, SavedChanged};

#[derive(Clone, Debug, Default)]
pub struct LikedTracks(Rc<RefCell<HashMap<String, bool>>>);

impl LikedTracks {
    pub fn learn(&self, tracks: &[Track]) {
        let mut liked = self.0.borrow_mut();
        for track in tracks {
            if let Some(saved) = track.saved {
                liked.entry(track.rri.id.clone()).or_insert(saved);
            }
        }
    }

    pub fn learn_liked(&self, tracks: &[Track]) {
        let mut liked = self.0.borrow_mut();
        for track in tracks {
            liked.entry(track.rri.id.clone()).or_insert(true);
        }
    }

    pub fn is_liked(&self, id: &str) -> bool {
        self.0.borrow().get(id).copied().unwrap_or(false)
    }

    pub fn apply(&self, event: &Event) -> bool {
        let Event::Saved(SavedChanged {
            item: ItemId::Tracks(ids),
            saved,
        }) = event
        else {
            return false;
        };
        let mut liked = self.0.borrow_mut();
        for id in ids {
            liked.insert(id.clone(), *saved);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{saved_tracks, track};

    fn liked(id: &str, saved: Option<bool>) -> Track {
        let mut track = track(id);
        track.saved = saved;
        track
    }

    #[test]
    fn liked_status_comes_with_loaded_tracks() {
        let liked_tracks = LikedTracks::default();
        liked_tracks.learn(&[
            liked("a", Some(true)),
            liked("b", Some(false)),
            liked("c", None),
        ]);
        assert!(liked_tracks.is_liked("a"));
        assert!(!liked_tracks.is_liked("b"));
        assert!(!liked_tracks.is_liked("c"));

        liked_tracks.learn_liked(&[track("c")]);
        assert!(liked_tracks.is_liked("c"));
    }

    #[test]
    fn a_late_page_does_not_undo_a_like() {
        let liked_tracks = LikedTracks::default();
        assert!(liked_tracks.apply(&saved_tracks(&["a"], true)));
        assert!(liked_tracks.apply(&saved_tracks(&["b"], false)));
        liked_tracks.learn(&[liked("a", Some(false)), liked("b", Some(true))]);
        assert!(liked_tracks.is_liked("a"));
        assert!(!liked_tracks.is_liked("b"));
        assert!(!liked_tracks.apply(&Event::PlayerSettingsChanged));
    }
}
