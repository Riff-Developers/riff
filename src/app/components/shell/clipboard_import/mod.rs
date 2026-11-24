use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gettextrs::gettext;
use gtk::prelude::*;
use libadwaita::prelude::*;

use crate::app::components::{dispatch_api_read, is_app_copied_link, EventListener};
use crate::app::{AppAction, AppModel, Dispatcher};
use riff_api::models::Link;

pub struct ClipboardImport {
    _watcher: Rc<ClipboardWatcher>,
}

impl ClipboardImport {
    pub fn new(
        window: libadwaita::ApplicationWindow,
        dispatcher: Dispatcher,
        app_model: Rc<AppModel>,
    ) -> Self {
        let watcher = Rc::new(ClipboardWatcher {
            window: window.clone(),
            dispatcher,
            app_model,
            dismissed: RefCell::new(None),
            dialog_open: Cell::new(false),
        });

        let watcher_weak = Rc::downgrade(&watcher);
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            if let Some(watcher) = watcher_weak.upgrade() {
                watcher.check_clipboard();
            }
        });

        Self { _watcher: watcher }
    }
}

impl EventListener for ClipboardImport {}

struct ClipboardWatcher {
    window: libadwaita::ApplicationWindow,
    dispatcher: Dispatcher,
    app_model: Rc<AppModel>,
    dismissed: RefCell<Option<String>>,
    dialog_open: Cell<bool>,
}

impl ClipboardWatcher {
    fn check_clipboard(self: &Rc<Self>) {
        if self.dialog_open.get() {
            return;
        }

        let clipboard = self.window.clipboard();
        let this = Rc::clone(self);
        clipboard.read_text_async(gio::Cancellable::NONE, move |result| {
            if let Ok(Some(text)) = result {
                this.handle_text(text.to_string());
            }
        });
    }

    fn handle_text(self: &Rc<Self>, text: String) {
        let dismissed = self.dismissed.borrow().clone();
        let decision = decide(&text, is_app_copied_link(&text), dismissed.as_deref());
        if let Some(link) = decision {
            debug!("clipboard contains a link: {link:?}");
            self.show_dialog(text, link);
        }
    }

    fn show_dialog(self: &Rc<Self>, text: String, link: Link) {
        if self.dialog_open.replace(true) {
            return;
        }

        let heading = gettext("Open Spotify link?");
        let body = match &link {
            // translators: shown when an album link is found in the clipboard.
            Link::Album(_) => {
                gettext("There’s a Spotify album link in your clipboard. Open it in Riff?")
            }
            // translators: shown when an artist link is found in the clipboard.
            Link::Artist(_) => {
                gettext("There’s a Spotify artist link in your clipboard. Open it in Riff?")
            }
            // translators: shown when a playlist link is found in the clipboard.
            Link::Playlist(_) => {
                gettext("There’s a Spotify playlist link in your clipboard. Open it in Riff?")
            }
            // translators: shown when a user profile link is found in the clipboard.
            Link::User(_) => {
                gettext("There’s a Spotify profile link in your clipboard. Open it in Riff?")
            }
            // translators: shown when a track link is found in the clipboard.
            Link::Track(_) => {
                gettext("There’s a Spotify track link in your clipboard. Open it in Riff?")
            }
        };

        let dialog = libadwaita::AlertDialog::new(Some(&heading), Some(&body));
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("open", &gettext("Open"));
        dialog.set_response_appearance("open", libadwaita::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("open"));
        dialog.set_close_response("cancel");

        let this = Rc::clone(self);
        dialog.choose(
            Some(&self.window),
            gio::Cancellable::NONE,
            move |response| {
                this.dialog_open.set(false);
                this.dismissed.replace(Some(text.trim().to_string()));
                if response.as_str() == "open" {
                    this.open_link(link);
                }
            },
        );
    }

    fn open_link(&self, link: Link) {
        match link {
            Link::Album(id) => self.dispatcher.dispatch(AppAction::ViewAlbum(id)),
            Link::Artist(id) => self.dispatcher.dispatch(AppAction::ViewArtist(id)),
            Link::Playlist(id) => self.dispatcher.dispatch(AppAction::ViewPlaylist(id)),
            Link::User(id) => self.dispatcher.dispatch(AppAction::ViewUser(id)),
            Link::Track(id) => {
                let api = self.app_model.api();
                dispatch_api_read(&self.dispatcher, move |tag| async move {
                    api.get_track(&id, tag).await.map(|song| {
                        AppAction::ViewAlbum(song.album.map(|a| a.rri.id).unwrap_or_default())
                    })
                });
            }
        }
    }
}

/// Pure decision logic: given the clipboard text, whether it matches a link
/// Riff itself copied, and the last dismissed link, return the link to prompt
/// for (if any).
fn decide(text: &str, is_app_copied: bool, dismissed: Option<&str>) -> Option<Link> {
    let link = riff_api::parse_link(text)?;
    if is_app_copied {
        return None;
    }
    if dismissed == Some(text.trim()) {
        return None;
    }
    Some(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decide_prompts_for_external_link() {
        let text = "https://open.spotify.com/album/abc";
        assert_eq!(
            decide(text, false, None),
            Some(Link::Album("abc".to_string()))
        );
    }

    #[test]
    fn test_decide_skips_app_copied_link() {
        let text = "https://open.spotify.com/track/abc";
        assert_eq!(decide(text, true, None), None);
    }

    #[test]
    fn test_decide_skips_dismissed_link() {
        let text = "https://open.spotify.com/track/abc";
        assert_eq!(decide(text, false, Some(text)), None);
    }

    #[test]
    fn test_decide_skips_non_spotify_text() {
        assert_eq!(decide("just some text", false, None), None);
        assert_eq!(decide("https://example.com", false, None), None);
    }

    #[test]
    fn test_decide_prompts_when_dismissed_is_different() {
        let text = "https://open.spotify.com/playlist/new";
        assert_eq!(
            decide(text, false, Some("https://open.spotify.com/playlist/old")),
            Some(Link::Playlist("new".to_string()))
        );
    }
}
