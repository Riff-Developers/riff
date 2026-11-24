use crate::app::state::*;
use riff_api::ApiService;
use std::cell::{Cell, Ref, RefCell};
use std::sync::Arc;

pub struct AppModel {
    state: RefCell<AppState>,
    api_service: Arc<ApiService>,
    started: Cell<bool>,
}

impl AppModel {
    pub fn new(state: AppState, api_service: Arc<ApiService>) -> Self {
        let state = RefCell::new(state);
        Self {
            state,
            api_service,
            started: Cell::new(false),
        }
    }

    pub fn api(&self) -> Arc<ApiService> {
        Arc::clone(&self.api_service)
    }

    pub fn get_state(&self) -> Ref<'_, AppState> {
        self.state.borrow()
    }

    pub fn map_state<T: 'static, F: FnOnce(&AppState) -> &T>(&self, map: F) -> Ref<'_, T> {
        Ref::map(self.state.borrow(), map)
    }

    pub fn update_state(&self, action: AppAction) -> Vec<AppEvent> {
        match action {
            AppAction::Core(action) => {
                let events = self.state.borrow_mut().apply(action);
                events.into_iter().map(app_event).collect()
            }
            action => {
                let mut started = self.started.get();
                let events = ui_events(action, &mut started);
                self.started.set(started);
                events
            }
        }
    }
}

pub trait ProvidesApi {
    fn api_service(&self) -> Arc<ApiService>;
}
