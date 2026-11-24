use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use crate::app::components::labels;
use crate::app::models::{Device as ConnectDevice, DeviceKind};
use crate::app::state::{DeviceEntry, DeviceList};
use gettextrs::gettext;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;

const VOLUME_DEBOUNCE: Duration = Duration::from_millis(250);

mod imp {

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/dev/diegovsky/Riff/components/device_selector.ui")]
    pub struct DeviceSelectorWidget {
        #[template_child]
        pub popover: TemplateChild<gtk::Popover>,

        #[template_child]
        pub current_icon: TemplateChild<gtk::Image>,

        #[template_child]
        pub current_name: TemplateChild<gtk::Label>,

        #[template_child]
        pub current_subtitle: TemplateChild<gtk::Label>,

        #[template_child]
        pub volume_box: TemplateChild<gtk::Box>,

        #[template_child]
        pub volume_scale: TemplateChild<gtk::Scale>,

        #[template_child]
        pub devices_list: TemplateChild<gtk::ListBox>,

        #[template_child]
        pub empty_label: TemplateChild<gtk::Label>,

        #[template_child]
        pub spinner: TemplateChild<gtk::Spinner>,

        pub row_targets: RefCell<Vec<Option<String>>>,
        pub syncing_volume: Cell<bool>,
        pub pending_volume: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceSelectorWidget {
        const NAME: &'static str = "DeviceSelectorWidget";
        type Type = super::DeviceSelectorWidget;
        type ParentType = gtk::Button;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DeviceSelectorWidget {
        fn constructed(&self) {
            self.parent_constructed();

            let popover: &gtk::Popover = &self.popover;
            popover.set_parent(&*self.obj());
            popover.set_autohide(true);

            self.obj().connect_clicked(clone!(
                #[weak]
                popover,
                move |_| popover.popup()
            ));
        }

        fn dispose(&self) {
            self.popover.unparent();
        }
    }

    impl WidgetImpl for DeviceSelectorWidget {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            self.popover.present();
        }
    }

    impl ButtonImpl for DeviceSelectorWidget {}
}

glib::wrapper! {
    pub struct DeviceSelectorWidget(ObjectSubclass<imp::DeviceSelectorWidget>) @extends gtk::Widget, gtk::Button,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

fn icon_for(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Phone => "phone-symbolic",
        DeviceKind::Computer => "computer-symbolic",
        DeviceKind::Speaker => "audio-speakers-symbolic",
        DeviceKind::Tv => "tv-symbolic",
        DeviceKind::Other => "audio-x-generic-symbolic",
    }
}

fn kind_label(kind: DeviceKind) -> String {
    match kind {
        DeviceKind::Phone => gettext("Phone"),
        DeviceKind::Computer => gettext("Computer"),
        DeviceKind::Speaker => gettext("Speaker"),
        DeviceKind::Tv => gettext("TV"),
        DeviceKind::Other => gettext("Device"),
    }
}

fn device_row(entry: &DeviceEntry) -> gtk::ListBoxRow {
    let (icon, label) = match entry {
        DeviceEntry::ThisDevice => ("computer-symbolic", gettext("This Device")),
        DeviceEntry::Other(ConnectDevice { kind, label, .. }) => (icon_for(*kind), label.clone()),
    };
    let content = gtk::Box::builder()
        .spacing(12)
        .margin_top(9)
        .margin_bottom(9)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(
        &gtk::Label::builder()
            .label(&label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(true)
        .build()
}

impl DeviceSelectorWidget {
    pub fn connect_switch_device<F>(&self, f: F)
    where
        F: Fn(Option<String>) + 'static,
    {
        self.imp().devices_list.connect_row_activated(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, row| {
                let target = this
                    .imp()
                    .row_targets
                    .borrow()
                    .get(row.index() as usize)
                    .cloned();
                if let Some(target) = target {
                    this.imp().popover.popdown();
                    f(target);
                }
            }
        ));
    }

    pub fn connect_volume_changed<F>(&self, f: F)
    where
        F: Fn(f64) + 'static,
    {
        let f = Rc::new(f);
        self.imp().volume_scale.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |scale| {
                let widget = this.imp();
                if widget.syncing_volume.get() {
                    return;
                }
                if let Some(pending) = widget.pending_volume.take() {
                    pending.remove();
                }
                let volume = scale.value() / 100.0;
                let f = f.clone();
                let source = glib::timeout_add_local_once(
                    VOLUME_DEBOUNCE,
                    clone!(
                        #[weak]
                        this,
                        move || {
                            this.imp().pending_volume.take();
                            f(volume);
                        }
                    ),
                );
                widget.pending_volume.replace(Some(source));
            }
        ));
    }

    pub fn update(&self, list: &DeviceList) {
        let widget = self.imp();
        widget.spinner.set_visible(!list.online);
        widget.spinner.set_spinning(!list.online);

        let (icon, name, subtitle) = match &list.current {
            DeviceEntry::ThisDevice => ("computer-symbolic", gettext("This Device"), None),
            DeviceEntry::Other(device) => (
                icon_for(device.kind),
                device.label.clone(),
                Some(kind_label(device.kind)),
            ),
        };
        widget.current_icon.set_icon_name(Some(icon));
        widget.current_name.set_label(&name);
        widget.current_subtitle.set_visible(subtitle.is_some());
        widget
            .current_subtitle
            .set_label(subtitle.as_deref().unwrap_or_default());

        self.set_icon_name(icon);
        if subtitle.is_some() {
            self.add_css_class("accent");
            // translators: Tooltip of the device button while another device plays. {} is its name.
            let tooltip = labels::fill(gettext("Playing on {}"), &[&name]);
            self.set_tooltip_text(Some(&tooltip));
        } else {
            self.remove_css_class("accent");
            self.set_tooltip_text(Some(&gettext("Connect to a Device")));
        }

        let volume = list.current_volume();
        widget.volume_box.set_visible(volume.is_some());
        if let Some(volume) = volume {
            if widget.pending_volume.borrow().is_none() {
                widget.syncing_volume.set(true);
                widget.volume_scale.set_value(volume as f64);
                widget.syncing_volume.set(false);
            }
        }

        widget.devices_list.remove_all();
        let mut targets = Vec::with_capacity(list.others.len());
        for entry in &list.others {
            widget.devices_list.append(&device_row(entry));
            targets.push(match entry {
                DeviceEntry::ThisDevice => None,
                DeviceEntry::Other(device) => Some(device.id.clone()),
            });
        }
        widget.row_targets.replace(targets);
        let empty = list.others.is_empty();
        widget.devices_list.set_visible(!empty);
        widget.empty_label.set_visible(empty);
    }
}
