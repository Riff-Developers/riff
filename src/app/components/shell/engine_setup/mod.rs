use riff_core::events::{EngineStatus, SessionChanged, SessionStatus};
use riff_playback::player::{Engine, ThisComputer};

use crate::app::{AppEvent, EventListener};

/// Whether playback on this computer is ready once logged in: always with
/// the built-in player, and once it says so with an engine the app runs.
pub fn playback_ready(session: &SessionChanged) -> bool {
    Engine::chosen() == Engine::Librespot || session.engine == EngineStatus::Ready
}

pub struct EngineSetup {
    this_computer: ThisComputer,
}

impl EngineSetup {
    pub fn new(this_computer: ThisComputer) -> Self {
        Self { this_computer }
    }

    fn logged_in(&self) {
        let engine = Engine::chosen();
        info!("Playing on this computer with {engine:?}");
        self.this_computer.set_engine(engine);
        self.this_computer
            .soloist()
            .set_logged_in(engine == Engine::Soloist);
    }

    fn logged_out(&self) {
        self.this_computer.soloist().set_logged_in(false);
    }
}

impl EventListener for EngineSetup {
    fn on_event(&mut self, event: &AppEvent) {
        let Some(session) = event.session() else {
            return;
        };
        if session.became(&SessionStatus::LoggedIn) {
            self.logged_in();
        } else if session.became(&SessionStatus::LoggedOut) {
            self.logged_out();
        }
    }
}
