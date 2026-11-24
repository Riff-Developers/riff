use std::rc::Rc;

use crate::app::components::{
    HeaderBarModel, HeaderImageShape, SimpleHeaderBarModel, SimpleHeaderBarModelWrapper,
};
use crate::app::models::{ArtistRef, ImageSet};
use crate::app::state::{LibraryAction, Pin, PinKind, PlaybackFields};
use crate::app::ProvidesApi;
use crate::app::{AppEvent, SongsSource};

use super::DetailsPageModel;

pub trait HasHeaderBarModel:
    SimpleHeaderBarModel + std::ops::Deref<Target = DetailsPageModel> + Sized + 'static
{
    fn to_headerbar_model(self: &Rc<Self>) -> Rc<impl HeaderBarModel + 'static> {
        Rc::new(SimpleHeaderBarModelWrapper::new(
            self.clone(),
            self.dispatcher.clone(),
        ))
    }
}

pub trait PageModel: ProvidesApi {
    fn apply_event(&self, _event: &AppEvent) {}

    fn get_title(&self) -> Option<String>;
    fn get_subtitle(&self) -> Option<String> {
        None
    }
    fn get_subtitle_detail(&self) -> Option<String> {
        None
    }
    fn get_artwork(&self) -> Option<ImageSet> {
        None
    }
    fn get_caption(&self) -> Option<String> {
        None
    }
    fn header_image_shape(&self) -> HeaderImageShape;
    fn default_icon(&self) -> Option<&str> {
        None
    }

    fn load_page_info(&self) {}
    fn load_more(&self) {}
    fn is_loaded(&self) -> bool {
        false
    }

    /// A page with a source gets play, queue and share buttons (see
    /// `impl_source_page!` and `impl_source_track_list!`).
    fn songs_source(&self) -> Option<SongsSource> {
        None
    }
    fn context_name(&self) -> Option<String> {
        self.get_title()
    }

    fn has_play_button(&self) -> bool {
        self.songs_source().is_some()
    }
    fn source_is_playing(&self) -> bool {
        false
    }
    fn start_play(&self, _id: &str) {}
    fn toggle_play(&self) {}
    fn shuffle_play(&self) {}

    fn has_queue_menu(&self) -> bool {
        self.songs_source().is_some()
    }
    fn queue_all(&self) {}

    /// Entries of the header's "⋮" menu besides "Add to Queue": (id, label).
    fn header_menu_entries(&self) -> Vec<(String, String)> {
        vec![]
    }
    fn on_header_menu(&self, _id: &str) {}

    fn has_like_button(&self) -> bool {
        false
    }
    fn is_liked(&self) -> bool {
        false
    }
    fn toggle_like(&self) {}
    fn like_visible(&self) -> bool {
        self.has_like_button()
    }
    fn like_tooltip(&self, _is_liked: bool) -> Option<String> {
        None
    }

    fn has_share_button(&self) -> bool {
        self.songs_source().and_then(|s| s.spotify_url()).is_some()
    }
    fn on_share_clicked(&self) {}

    /// Returns a list of clickable links for the subtitle area.
    /// Each entry is rendered as an independent button.
    /// Returns an empty vec if the subtitle should not be clickable.
    fn get_subtitle_links(&self) -> Vec<ArtistRef> {
        vec![]
    }

    /// Navigate to a subtitle link target by ID (e.g. artist page or user page).
    fn navigate_to_subtitle_link(&self, _id: &str) {}

    /// Returns true if this event means page details should be refreshed.
    fn should_refresh_details(&self, event: &AppEvent) -> bool;
    /// Returns true if this event means liked state changed.
    fn should_refresh_liked(&self, _event: &AppEvent) -> bool {
        false
    }
}

pub trait PinnedPageModel: PageModel + std::ops::Deref<Target = DetailsPageModel> {
    fn pin_kind(&self) -> PinKind {
        PinKind::Playlist
    }

    /// The store id of the object this page pins. Defaults to the page's own
    /// id; override for pages whose target is dynamic (e.g. the currently
    /// playing song on the now-playing page).
    fn pinned_object_id(&self) -> Option<String> {
        Some(self.id.clone())
    }

    /// Whether this page type supports a pin control (independent of feature flag).
    fn supports_pin_button(&self) -> bool {
        false
    }

    fn is_pinned(&self) -> bool {
        self.pinned_object_id().is_some_and(|id| {
            self.app_model
                .get_state()
                .library
                .is_pinned(self.pin_kind(), &id)
        })
    }

    fn toggle_pin(&self) {
        let Some(id) = self.pinned_object_id() else {
            return;
        };
        let action = if self.is_pinned() {
            LibraryAction::Unpin(self.pin_kind(), id)
        } else if self.is_liked() {
            LibraryAction::Pin(Pin::new(self.pin_kind(), id, self.get_title()))
        } else {
            return;
        };
        self.dispatcher.dispatch(action.into());
    }
}

/// Check if a playback event means we should update the play button.
/// Returns whether playback is playing when the status or the track changed,
/// `None` otherwise.
pub fn is_playback_event(event: &AppEvent) -> Option<bool> {
    event
        .playback()
        .filter(|p| {
            p.changed
                .intersects(PlaybackFields::STATUS | PlaybackFields::TRACK)
        })
        .map(|p| p.state.is_playing())
}
