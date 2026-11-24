use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use riff_api::{ApiService, DomainError, Load};
use riff_core::events::Event;
use riff_core::load;
use riff_core::screens::{PageFields, Screen, Update};
use riff_core::session::SessionAction;

use crate::app::components::error_actions;
use crate::app::state::Action;
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher};

type OnChange<S> = Rc<dyn Fn(&S, PageFields)>;
type Refused<S> = Rc<RefCell<Vec<(<S as Screen>::Request, Load)>>>;

pub struct ScreenModel<S: Screen> {
    screen: Rc<RefCell<S>>,
    api: Arc<ApiService>,
    dispatcher: Dispatcher,
    on_change: OnChange<S>,
    refused: Refused<S>,
}

impl<S: Screen> Clone for ScreenModel<S> {
    fn clone(&self) -> Self {
        Self {
            screen: Rc::clone(&self.screen),
            api: Arc::clone(&self.api),
            dispatcher: self.dispatcher.clone(),
            on_change: Rc::clone(&self.on_change),
            refused: Rc::clone(&self.refused),
        }
    }
}

impl<S: Screen + 'static> ScreenModel<S> {
    pub fn new(
        screen: S,
        app_model: &AppModel,
        dispatcher: Dispatcher,
        on_change: impl Fn(&S, PageFields) + 'static,
    ) -> Self {
        Self {
            screen: Rc::new(RefCell::new(screen)),
            api: app_model.api(),
            dispatcher,
            on_change: Rc::new(on_change),
            refused: Rc::default(),
        }
    }

    pub fn get(&self) -> Ref<'_, S> {
        self.screen.borrow()
    }

    pub fn open(&self) {
        self.update(S::open);
    }

    pub fn load_more(&self) {
        self.update(S::load_more);
    }

    pub fn update(&self, change: impl FnOnce(&mut S) -> Vec<S::Request>) {
        let requests = change(&mut self.screen.borrow_mut());
        self.run_all(requests);
    }

    pub fn apply(&self, event: &AppEvent) {
        if let AppEvent::Core(Event::TokenRefreshed) = event {
            let refused = std::mem::take(&mut *self.refused.borrow_mut());
            for (request, load) in refused {
                self.run(request, load, true);
            }
        }
        if let AppEvent::Core(event) = event {
            let update = self.screen.borrow_mut().apply(event);
            self.handle(update);
        }
    }

    fn run_all(&self, requests: Vec<S::Request>) {
        let load = load::visible();
        for request in requests {
            self.run(request, load, false);
        }
    }

    fn run(&self, request: S::Request, load: Load, retried: bool) {
        let future = self
            .screen
            .borrow()
            .fetch(Arc::clone(&self.api), request.clone(), load);
        let task = tokio::spawn(future);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let Ok(result) = task.await else {
                return;
            };
            if matches!(result, Err(DomainError::AuthExpired)) && !retried {
                // Sent again once the token is renewed.
                this.refused.borrow_mut().push((request, load));
                this.dispatcher.dispatch(AppAction::Core(Action::Session(
                    SessionAction::TokenRejected,
                )));
                return;
            }
            let update = this.screen.borrow_mut().receive(request, result);
            this.handle(update);
        });
    }

    fn handle(&self, update: Update<S::Request>) {
        let Update {
            changed,
            title,
            actions,
            requests,
            error,
        } = update;
        if !changed.is_empty() {
            (self.on_change)(&self.screen.borrow(), changed);
        }
        let route = self.screen.borrow().route();
        let mut dispatched = vec![];
        if let Some(title) = title {
            dispatched.push(AppAction::Core(Action::RecordTitle(route.clone(), title)));
        }
        dispatched.extend(actions.into_iter().map(AppAction::from));
        if !changed.is_empty() {
            dispatched.push(AppAction::PageLoaded(route, changed));
        }
        if let Some(error) = error {
            dispatched.extend(error_actions(error));
        }
        if !dispatched.is_empty() {
            self.dispatcher.dispatch_many(dispatched);
        }
        self.run_all(requests);
    }
}
