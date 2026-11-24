use riff_playback::player::{Engine, ThisComputer};

use crate::app::state::{LoginMethod, SessionAction};
use crate::app::{AppAction, Dispatcher};

pub struct LoginModel {
    dispatcher: Dispatcher,
    this_computer: ThisComputer,
}

impl LoginModel {
    pub fn new(dispatcher: Dispatcher, this_computer: ThisComputer) -> Self {
        Self {
            dispatcher,
            this_computer,
        }
    }

    pub fn try_autologin(&self) {
        self.dispatcher
            .dispatch(SessionAction::TryLogin(LoginMethod::Saved).into());
    }

    pub fn log_in_with(&self, engine: Engine) {
        engine.choose();
        if engine == Engine::Soloist {
            self.this_computer.soloist().prepare();
        }
        self.log_in();
    }

    pub fn set_key(&self, key: String) {
        self.this_computer.soloist().set_key(key);
    }

    pub fn retry_login(&self) {
        self.log_in();
    }

    pub fn cancel(&self) {
        self.dispatcher.dispatch(SessionAction::Logout.into());
    }

    pub fn notify(&self, text: String) {
        self.dispatcher.dispatch(AppAction::ShowNotification(text));
    }

    fn log_in(&self) {
        self.dispatcher
            .dispatch(SessionAction::TryLogin(LoginMethod::Browser).into())
    }
}
