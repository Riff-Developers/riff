use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use libadwaita::prelude::*;

use riff_config::setting::Unavailable;
use riff_config::{Setting, SettingsStore, SettingsSupport};

/// Why a listed choice can't be picked, as told to the user.
fn reason(why: Unavailable) -> String {
    match why {
        Unavailable::LegalRestrictions => {
            gettextrs::gettext("Unsupported due to legal restrictions.")
        }
    }
}

#[derive(Debug, Clone)]
struct Listed {
    value: &'static str,
    unavailable: Option<Unavailable>,
}

#[derive(Debug)]
pub struct ChoiceRow {
    setting: Setting,
    row: libadwaita::ComboRow,
    entries: Vec<(&'static str, String)>,
    listed: Rc<RefCell<Vec<Listed>>>,
}

impl ChoiceRow {
    /// `values` are the stored values of the labels the template gave `row`,
    /// in the same order.
    pub fn new(setting: Setting, row: &libadwaita::ComboRow, values: &[&'static str]) -> Self {
        let labels = row
            .model()
            .and_downcast::<gtk::StringList>()
            .expect("a choice row lists its labels in a StringList");
        assert_eq!(
            labels.n_items() as usize,
            values.len(),
            "{setting:?} needs a value for each label"
        );
        let entries = values
            .iter()
            .enumerate()
            .map(|(i, &value)| (value, labels.string(i as u32).unwrap_or_default().into()))
            .collect();
        let this = Self {
            setting,
            row: row.clone(),
            entries,
            listed: Rc::default(),
        };
        this.install_factory();
        this
    }

    pub fn setting(&self) -> Setting {
        self.setting
    }

    /// The stored value of the choice shown.
    pub fn selected_value(&self) -> Option<&'static str> {
        self.listed
            .borrow()
            .get(self.row.selected() as usize)
            .map(|listed| listed.value)
    }

    /// Lists the choices `support` offers, and shows the stored one.
    pub fn apply(&self, support: &SettingsSupport, settings: &SettingsStore) {
        SettingsStore::unbind(&self.row, "selected");

        let listed: Vec<Listed> = match support.choices(self.setting) {
            Some(choices) => choices
                .iter()
                .filter(|choice| self.entries.iter().any(|(v, _)| *v == choice.value))
                .map(|choice| Listed {
                    value: choice.value,
                    unavailable: choice.unavailable,
                })
                .collect(),
            None => self
                .entries
                .iter()
                .map(|&(value, _)| Listed {
                    value,
                    unavailable: None,
                })
                .collect(),
        };
        let labels: Vec<&str> = listed
            .iter()
            .filter_map(|listed| {
                self.entries
                    .iter()
                    .find(|(value, _)| *value == listed.value)
                    .map(|(_, label)| label.as_str())
            })
            .collect();
        *self.listed.borrow_mut() = listed;
        self.row.set_model(Some(&gtk::StringList::new(&labels)));

        let shown = Rc::clone(&self.listed);
        let saved = Rc::clone(&self.listed);
        settings
            .bind(self.setting, &self.row, "selected")
            .mapping(move |variant, _| {
                let stored = variant.str()?;
                let listed = shown.borrow();
                let can_pick = |listed: &Listed| listed.unavailable.is_none();
                let position = listed
                    .iter()
                    .position(|l| l.value == stored && can_pick(l))
                    .or_else(|| listed.iter().position(can_pick))
                    .unwrap_or(0);
                Some((position as u32).to_value())
            })
            .set_mapping(move |value, _| {
                let position = value.get::<u32>().ok()?;
                let listed = saved.borrow();
                let listed = listed.get(position as usize)?;
                listed
                    .unavailable
                    .is_none()
                    .then(|| listed.value.to_variant())
            })
            .build();
    }

    /// Shows choices that can't be picked dimmed, with a warning saying why,
    /// and keeps them from being picked. Only the popup list uses it; the row
    /// shows the choice picked as usual.
    fn install_factory(&self) {
        let factory = gtk::SignalListItemFactory::new();

        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_hexpand(true);

            let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
            warning.set_visible(false);

            row.append(&label);
            row.append(&warning);
            item.set_child(Some(&row));
        });

        let listed = Rc::clone(&self.listed);
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(label) = row.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(warning) = row.last_child().and_downcast::<gtk::Image>() else {
                return;
            };

            let text = item
                .item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default();
            label.set_label(&text);

            let unavailable = listed
                .borrow()
                .get(item.position() as usize)
                .and_then(|listed| listed.unavailable);
            match unavailable {
                Some(why) => {
                    label.add_css_class("dim-label");
                    warning.set_visible(true);
                    row.set_tooltip_text(Some(&reason(why)));
                }
                None => {
                    label.remove_css_class("dim-label");
                    warning.set_visible(false);
                    row.set_tooltip_text(None);
                }
            }
            item.set_selectable(unavailable.is_none());
            item.set_activatable(unavailable.is_none());
        });

        self.row.set_list_factory(Some(&factory));
    }
}
