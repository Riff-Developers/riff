use std::rc::Rc;

use gtk::prelude::Cast;

use crate::app::components::{Component, EventListener};
use crate::app::state::{DeviceList, PlaybackAction, PlaybackFields};
use crate::app::{AppEvent, AppModel, Dispatcher};

use super::widget::DeviceSelectorWidget;

pub struct DeviceSelectorModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
}

impl DeviceSelectorModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    pub fn device_list(&self) -> DeviceList {
        self.app_model.get_state().playback.device_list()
    }

    /// Moves playback to a device; `None` is this one.
    pub fn transfer_to(&self, id: Option<String>) {
        self.dispatcher
            .dispatch(PlaybackAction::TransferTo(id).into());
    }

    pub fn set_volume(&self, volume: f64) {
        self.dispatcher
            .dispatch(PlaybackAction::SetVolume(volume).into());
    }
}

pub struct DeviceSelector {
    widget: DeviceSelectorWidget,
    model: Rc<DeviceSelectorModel>,
}

impl DeviceSelector {
    pub fn new(widget: DeviceSelectorWidget, model: DeviceSelectorModel) -> Self {
        let model = Rc::new(model);

        widget.connect_switch_device(clone!(
            #[weak]
            model,
            move |id| {
                model.transfer_to(id);
            }
        ));

        widget.connect_volume_changed(clone!(
            #[weak]
            model,
            move |volume| {
                model.set_volume(volume);
            }
        ));

        widget.update(&model.device_list());

        Self { widget, model }
    }
}

impl Component for DeviceSelector {
    fn get_root_widget(&self) -> &gtk::Widget {
        self.widget.upcast_ref()
    }
}

impl EventListener for DeviceSelector {
    fn on_event(&mut self, event: &AppEvent) {
        if event.devices().is_some() || event.playback_changed(PlaybackFields::VOLUME) {
            self.widget.update(&self.model.device_list());
        }
    }
}
