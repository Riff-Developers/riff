use gettextrs::gettext;
use gtk::prelude::*;
use std::rc::Rc;

use super::NowPlayingModel;
use crate::app::components::{
    Component, DetailsPageComponent, EventListener, HasHeaderBarModel, PageHeader, QueueList,
};
use crate::app::state::PlaybackFields;
use crate::app::AppEvent;

pub struct NowPlaying {
    stack: gtk::Stack,
    component: DetailsPageComponent<NowPlayingModel>,
    #[allow(dead_code)]
    status_page: libadwaita::StatusPage,
}

impl NowPlaying {
    pub fn new(model: Rc<NowPlayingModel>, header: PageHeader) -> Self {
        let mut component =
            DetailsPageComponent::new(model.clone(), model.to_headerbar_model(), header);
        let queue_list = QueueList::new(model.queue_list_model());
        component.append_content(queue_list.widget());
        queue_list.connect_scrolling();
        component.add_child(Box::new(queue_list));

        let status_page = libadwaita::StatusPage::new();
        status_page.set_title(&gettext("No Track Playing"));
        status_page.set_icon_name(Some("audio-x-generic-symbolic"));

        let stack = gtk::Stack::new();
        stack.add_named(component.get_root_widget(), Some("content"));
        stack.add_named(&status_page, Some("empty"));

        let visible = if model.has_queue() {
            "content"
        } else {
            "empty"
        };
        stack.set_visible_child_name(visible);

        Self {
            stack,
            component,
            status_page,
        }
    }

    fn update_empty_state(&self) {
        let name = if self.component.model().has_queue() {
            "content"
        } else {
            "empty"
        };
        self.stack.set_visible_child_name(name);
    }
}

impl Component for NowPlaying {
    fn get_root_widget(&self) -> &gtk::Widget {
        self.stack.upcast_ref()
    }
    fn get_children(&mut self) -> Option<&mut Vec<Box<dyn EventListener>>> {
        self.component.get_children()
    }
}

impl EventListener for NowPlaying {
    fn on_event(&mut self, event: &AppEvent) {
        self.component.handle_event(event);
        if event.queue_changed() || event.playback_changed(PlaybackFields::TRACK) {
            self.update_empty_state();
        }
        self.broadcast_event(event);
    }
}
