use futures::channel::mpsc::UnboundedSender;
use std::rc::Rc;

use riff_mpris::{Command, Mpris};

use crate::app::components::EventListener;
use crate::app::{AppAction, AppEvent, AppModel};

pub struct MprisListener {
    app_model: Rc<AppModel>,
    mpris: Mpris,
}

impl MprisListener {
    pub fn start(app_model: Rc<AppModel>, sender: UnboundedSender<AppAction>) -> Self {
        let mpris = riff_mpris::start(move |command| {
            let action = match command {
                Command::Playback(action) => action.into(),
                Command::Raise => AppAction::Raise,
            };
            sender.unbounded_send(action).is_ok()
        });
        Self { app_model, mpris }
    }
}

impl EventListener for MprisListener {
    fn on_event(&mut self, event: &AppEvent) {
        if let Some(event) = event.playback() {
            let state = self.app_model.get_state();
            self.mpris.on_playback_changed(event, &state.playback);
        } else if event.queue_changed() {
            let state = self.app_model.get_state();
            self.mpris.on_queue_changed(&state.playback);
        }
    }
}
