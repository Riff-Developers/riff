use std::rc::Rc;

use crate::app::components::EventListener;
use crate::app::state::{LibraryAction, LibraryFields, SessionStatus};
use crate::app::{AppEvent, AppModel, Dispatcher};
use riff_config::setting::{PinnedKind, PinnedObject};
use riff_config::SettingsStore;
use riff_core::library::{Pin, PinKind};

pub struct PinStore {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
    settings: SettingsStore,
}

impl PinStore {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
            settings: SettingsStore::new(),
        }
    }

    fn load(&self) {
        let Some(user) = self.app_model.get_state().session.user.clone() else {
            return;
        };
        let pins = self.settings.pins(&user).into_iter().map(pin).collect();
        self.dispatcher
            .dispatch(LibraryAction::PinsLoaded(pins).into());
    }

    fn save(&self) {
        let state = self.app_model.get_state();
        let pins = state.library.pins();
        let Some(user) = state.session.user.as_deref().filter(|_| pins.is_loaded()) else {
            return;
        };
        self.settings
            .set_pins(user, pins.iter().map(pinned_object).collect());
    }
}

impl EventListener for PinStore {
    fn on_event(&mut self, event: &AppEvent) {
        if event
            .session()
            .is_some_and(|session| session.became(&SessionStatus::LoggedIn))
        {
            self.load();
        }
        if event.library_changed(LibraryFields::PINS) {
            self.save();
        }
    }
}

fn pin(object: PinnedObject) -> Pin {
    let kind = match object.kind {
        PinnedKind::Playlist => PinKind::Playlist,
        PinnedKind::Album => PinKind::Album,
        PinnedKind::Artist => PinKind::Artist,
        PinnedKind::Track => PinKind::Track,
    };
    Pin::new(kind, object.id, object.title)
}

fn pinned_object(pin: &Pin) -> PinnedObject {
    let kind = match pin.kind {
        PinKind::Playlist => PinnedKind::Playlist,
        PinKind::Album => PinnedKind::Album,
        PinKind::Artist => PinnedKind::Artist,
        PinKind::Track => PinnedKind::Track,
    };
    PinnedObject {
        title: pin.title.clone(),
        ..PinnedObject::new(pin.id.clone(), kind)
    }
}
