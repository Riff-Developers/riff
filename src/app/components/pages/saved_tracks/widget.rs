use std::rc::Rc;

use super::SavedTracksModel;
use crate::app::components::{
    Component, DetailsPageComponent, EventListener, HasHeaderBarModel, PageHeader,
};
use crate::app::state::SessionStatus;
use crate::app::AppEvent;

pub struct SavedTracks {
    model: Rc<SavedTracksModel>,
    component: DetailsPageComponent<SavedTracksModel>,
}

impl SavedTracks {
    pub fn new(model: Rc<SavedTracksModel>, header: PageHeader) -> Self {
        let mut component =
            DetailsPageComponent::new(model.clone(), model.to_headerbar_model(), header);
        component.create_track_list(None);

        Self { model, component }
    }
}

impl Component for SavedTracks {
    fn get_root_widget(&self) -> &gtk::Widget {
        self.component.get_root_widget()
    }
    fn get_children(&mut self) -> Option<&mut Vec<Box<dyn EventListener>>> {
        self.component.get_children()
    }
}

impl EventListener for SavedTracks {
    fn on_event(&mut self, event: &AppEvent) {
        if event
            .session()
            .is_some_and(|s| s.became(&SessionStatus::LoggedIn))
        {
            self.model.load_initial();
        }
        self.component.handle_event(event);
        self.broadcast_event(event);
    }
}
