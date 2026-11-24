use crate::app::state::Action;
use crate::app::state::EntryId;
use crate::app::state::NavigationAction;
use crate::app::{AppAction, AppModel, Dispatcher};
use std::rc::Rc;

pub struct NavigationModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
}

impl NavigationModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    pub fn set_nav_hidden(&self, hidden: bool) {
        self.dispatcher
            .dispatch(AppAction::Core(Action::SetNavigationHidden(hidden)));
    }

    pub fn go_back(&self) {
        self.dispatcher.dispatch(NavigationAction::Back.into());
    }

    pub fn go_forward(&self) {
        if self
            .app_model
            .get_state()
            .browser
            .navigator()
            .can_go_forward()
        {
            self.dispatcher.dispatch(NavigationAction::Forward.into());
        }
    }

    pub fn save_scroll(&self, entry: EntryId, scroll: f64) {
        self.dispatcher
            .dispatch(NavigationAction::SaveScroll(entry, scroll.max(0.0) as u32).into());
    }
}
