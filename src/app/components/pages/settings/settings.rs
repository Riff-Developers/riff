use crate::app::components::EventListener;
use crate::app::AppEvent;
use riff_config::feature_flag::FeatureFlag;

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;
use libadwaita::prelude::*;

use super::choice_row::ChoiceRow;
use super::EqualizerWidget;
use super::PanWidget;
use super::PitchWidget;
use super::SettingsModel;

use riff_config::{Setting, SettingsStore, SettingsSupport};

const CHOICES: &[(Setting, &[&str])] = &[
    (Setting::PlayerBitrate, &["96", "160", "320", "lossless"]),
    (Setting::AudioBackend, &["pulseaudio", "alsa"]),
    (
        Setting::AudioFormat,
        &["s16", "s24", "s24_3", "s32", "f32", "f64"],
    ),
    (Setting::VolumeCurve, &["log", "linear", "cubic"]),
    (Setting::NormalisationType, &["auto", "track", "album"]),
    (Setting::NormalisationMethod, &["dynamic", "basic"]),
];

#[derive(Debug)]
pub struct NumberRow {
    setting: Setting,
    adjustment: gtk::Adjustment,
    lower: f64,
    upper: f64,
    unsigned: bool,
    on_change: glib::SignalHandlerId,
}

impl NumberRow {
    fn stored(&self, settings: &SettingsStore) -> f64 {
        if self.unsigned {
            f64::from(settings.uint(self.setting))
        } else {
            settings.double(self.setting)
        }
    }

    /// Narrows the range to what `support` takes and shows the stored value
    /// in it, without saving over it.
    fn apply(&self, support: &SettingsSupport, settings: &SettingsStore) {
        let (lower, upper) = support
            .range(self.setting)
            .map_or((self.lower, self.upper), |(min, max)| {
                (min.max(self.lower), max.min(self.upper))
            });
        self.adjustment.block_signal(&self.on_change);
        self.adjustment.set_lower(lower);
        self.adjustment.set_upper(upper);
        self.adjustment
            .set_value(self.stored(settings).clamp(lower, upper));
        self.adjustment.unblock_signal(&self.on_change);
    }
}

mod imp {

    use super::*;
    use libadwaita::subclass::prelude::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/dev/diegovsky/Riff/components/settings.ui")]
    pub struct SettingsDialog {
        #[template_child]
        pub player_bitrate: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub alsa_device: TemplateChild<gtk::Entry>,

        #[template_child]
        pub alsa_device_row: TemplateChild<libadwaita::ActionRow>,

        #[template_child]
        pub audio_backend: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub gapless_playback: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub ap_port: TemplateChild<gtk::Entry>,

        #[template_child]
        pub ap_port_row: TemplateChild<libadwaita::ActionRow>,

        #[template_child]
        pub audio_group: TemplateChild<libadwaita::PreferencesGroup>,

        #[template_child]
        pub network_group: TemplateChild<libadwaita::PreferencesGroup>,

        #[template_child]
        pub theme: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub queue_side_panel_switch: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub close_behavior: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub inhibit_suspend_switch: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub skip_explicit_switch: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub volume_curve: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub mono_audio_switch: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub audio_format: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub normalisation_group: TemplateChild<libadwaita::PreferencesGroup>,

        #[template_child]
        pub normalisation_switch: TemplateChild<libadwaita::SwitchRow>,

        #[template_child]
        pub normalisation_type: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub normalisation_method: TemplateChild<libadwaita::ComboRow>,

        #[template_child]
        pub normalisation_pregain: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub normalisation_threshold: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub normalisation_attack: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub normalisation_release: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub normalisation_knee: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub equalizer: TemplateChild<EqualizerWidget>,

        #[template_child]
        pub pan: TemplateChild<PanWidget>,

        #[template_child]
        pub pitch: TemplateChild<PitchWidget>,

        #[template_child]
        pub memory_cache_size: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub disk_cache_size: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub audio_cache_size: TemplateChild<libadwaita::SpinRow>,

        #[template_child]
        pub clear_cache_button: TemplateChild<gtk::Button>,

        pub support: RefCell<SettingsSupport>,
        pub choices: RefCell<Vec<ChoiceRow>>,
        pub numbers: RefCell<Vec<NumberRow>>,
        pub watcher: OnceCell<SettingsStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsDialog {
        const NAME: &'static str = "SettingsWindow";
        type Type = super::SettingsDialog;
        type ParentType = libadwaita::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SettingsDialog {}
    impl WidgetImpl for SettingsDialog {}
    impl AdwDialogImpl for SettingsDialog {}
    impl PreferencesDialogImpl for SettingsDialog {}
}

glib::wrapper! {
    pub struct SettingsDialog(ObjectSubclass<imp::SettingsDialog>) @extends gtk::Widget, libadwaita::Dialog, libadwaita::PreferencesDialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SettingsDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsDialog {
    pub fn new() -> Self {
        let dialog: Self = glib::Object::new();

        dialog.bind_settings();
        dialog.bind_choices();
        dialog.bind_feature_flags();
        dialog.connect_theme_select();
        dialog.watch_visibility();
        dialog
    }

    fn bind_settings(&self) {
        let widget = self.imp();
        let settings = SettingsStore::new();

        let bind_adjustment = |setting: Setting, adjustment: &gtk::Adjustment, unsigned: bool| {
            let saved = settings.clone();
            let on_change = adjustment.connect_value_changed(move |adj| {
                let _ = if unsigned {
                    saved.set_uint(setting, adj.value() as u32)
                } else {
                    saved.set_double(setting, adj.value())
                };
            });
            let number = NumberRow {
                setting,
                adjustment: adjustment.clone(),
                lower: adjustment.lower(),
                upper: adjustment.upper(),
                unsigned,
                on_change,
            };
            number.apply(&SettingsSupport::none(), &settings);
            widget.numbers.borrow_mut().push(number);
        };
        let bind_double_adjustment = |setting: Setting, adjustment: &gtk::Adjustment| {
            bind_adjustment(setting, adjustment, false)
        };
        let bind_uint_adjustment = |setting: Setting, adjustment: &gtk::Adjustment| {
            bind_adjustment(setting, adjustment, true)
        };

        let alsa_device = widget.alsa_device.downcast_ref::<gtk::Entry>().unwrap();
        settings
            .bind(Setting::AlsaDevice, alsa_device, "text")
            .build();

        let gapless_playback = widget
            .gapless_playback
            .downcast_ref::<libadwaita::SwitchRow>()
            .unwrap();
        settings
            .bind(Setting::GaplessPlayback, gapless_playback, "active")
            .build();

        let ap_port = widget.ap_port.downcast_ref::<gtk::Entry>().unwrap();
        settings
            .bind(Setting::ApPort, ap_port, "text")
            .mapping(|variant, _| variant.get::<u32>().map(|s| s.to_value()))
            .set_mapping(|value, _| value.get::<u32>().ok().map(|u| u.to_variant()))
            .build();

        let mono_audio_switch = widget
            .mono_audio_switch
            .downcast_ref::<libadwaita::SwitchRow>()
            .unwrap();
        settings
            .bind(Setting::MonoAudio, mono_audio_switch, "active")
            .build();

        let normalisation_switch = widget
            .normalisation_switch
            .downcast_ref::<libadwaita::SwitchRow>()
            .unwrap();
        settings
            .bind(Setting::Normalisation, normalisation_switch, "active")
            .build();

        let normalisation_pregain = widget
            .normalisation_pregain
            .downcast_ref::<libadwaita::SpinRow>()
            .unwrap();
        bind_double_adjustment(
            Setting::NormalisationPregainDb,
            &normalisation_pregain.adjustment(),
        );

        let normalisation_threshold = widget
            .normalisation_threshold
            .downcast_ref::<libadwaita::SpinRow>()
            .unwrap();
        bind_double_adjustment(
            Setting::NormalisationThresholdDbfs,
            &normalisation_threshold.adjustment(),
        );

        let normalisation_attack = widget
            .normalisation_attack
            .downcast_ref::<libadwaita::SpinRow>()
            .unwrap();
        bind_double_adjustment(
            Setting::NormalisationAttackMs,
            &normalisation_attack.adjustment(),
        );

        let normalisation_release = widget
            .normalisation_release
            .downcast_ref::<libadwaita::SpinRow>()
            .unwrap();
        bind_double_adjustment(
            Setting::NormalisationReleaseMs,
            &normalisation_release.adjustment(),
        );

        let normalisation_knee = widget
            .normalisation_knee
            .downcast_ref::<libadwaita::SpinRow>()
            .unwrap();
        bind_double_adjustment(
            Setting::NormalisationKneeDb,
            &normalisation_knee.adjustment(),
        );

        let theme = widget.theme.downcast_ref::<libadwaita::ComboRow>().unwrap();
        settings
            .bind(Setting::ThemePreference, theme, "selected")
            .mapping(|variant, _| {
                variant.str().map(|s| {
                    match s {
                        "light" => 0,
                        "dark" => 1,
                        "system" => 2,
                        _ => unreachable!(),
                    }
                    .to_value()
                })
            })
            .set_mapping(|value, _| {
                value.get::<u32>().ok().map(|u| {
                    match u {
                        0 => "light",
                        1 => "dark",
                        2 => "system",
                        _ => unreachable!(),
                    }
                    .to_variant()
                })
            })
            .build();

        settings
            .bind(
                Setting::QueueSidePanel,
                &*widget.queue_side_panel_switch,
                "active",
            )
            .build();

        let close_behavior = widget
            .close_behavior
            .downcast_ref::<libadwaita::ComboRow>()
            .unwrap();
        settings
            .bind(Setting::CloseWindowBehavior, close_behavior, "selected")
            .mapping(|variant, _| {
                variant.str().map(|s| {
                    match s {
                        "ask" => 0,
                        "minimize-to-background" => 1,
                        "stop-and-quit" => 2,
                        _ => unreachable!(),
                    }
                    .to_value()
                })
            })
            .set_mapping(|value, _| {
                value.get::<u32>().ok().map(|u| {
                    match u {
                        0 => "ask",
                        1 => "minimize-to-background",
                        2 => "stop-and-quit",
                        _ => unreachable!(),
                    }
                    .to_variant()
                })
            })
            .build();

        let inhibit_suspend_switch = widget
            .inhibit_suspend_switch
            .downcast_ref::<libadwaita::SwitchRow>()
            .expect("inhibit_suspend_switch must be a SwitchRow");
        settings
            .bind(Setting::InhibitSuspend, inhibit_suspend_switch, "active")
            .build();

        let skip_explicit_switch = widget
            .skip_explicit_switch
            .downcast_ref::<libadwaita::SwitchRow>()
            .expect("skip_explicit_switch must be a SwitchRow");
        settings
            .bind(Setting::SkipExplicit, skip_explicit_switch, "active")
            .build();

        bind_uint_adjustment(
            Setting::MemoryCacheSizeMb,
            &widget.memory_cache_size.adjustment(),
        );
        bind_uint_adjustment(
            Setting::DiskCacheSizeMb,
            &widget.disk_cache_size.adjustment(),
        );
        bind_uint_adjustment(
            Setting::AudioCacheSizeMb,
            &widget.audio_cache_size.adjustment(),
        );
    }

    fn bind_feature_flags(&self) {
        let settings = SettingsStore::new();
        let group = libadwaita::PreferencesGroup::new();
        group.set_title(&gettextrs::gettext("Experimental Features"));
        group.set_description(Some(&gettextrs::gettext(
            "These features are still in development and may be unstable or incomplete.",
        )));

        for flag in FeatureFlag::ALL.iter() {
            let row = libadwaita::SwitchRow::new();
            row.set_title(flag.title());
            row.set_subtitle(flag.description());
            settings.bind(flag.setting(), &row, "active").build();
            group.add(&row);
        }

        let page = self
            .upcast_ref::<libadwaita::PreferencesDialog>()
            .visible_page()
            .unwrap();
        page.add(&group);
    }

    /// Lists in each choice row the choices of its setting.
    fn bind_choices(&self) {
        let widget = self.imp();
        let rows = CHOICES.iter().map(|&(setting, values)| {
            let row = match setting {
                Setting::PlayerBitrate => &widget.player_bitrate,
                Setting::AudioBackend => &widget.audio_backend,
                Setting::AudioFormat => &widget.audio_format,
                Setting::VolumeCurve => &widget.volume_curve,
                Setting::NormalisationType => &widget.normalisation_type,
                Setting::NormalisationMethod => &widget.normalisation_method,
                _ => unreachable!("{:?} has no choice row", setting),
            };
            ChoiceRow::new(setting, row, values)
        });
        widget.choices.borrow_mut().extend(rows);
        self.apply_support(SettingsSupport::none());
    }

    /// The rows and groups of settings that depend on the player, and the
    /// settings that show them: one is shown if any of its settings is
    /// supported.
    fn player_rows(&self) -> Vec<(&'static [Setting], gtk::Widget)> {
        let widget = self.imp();
        vec![
            (
                &[Setting::PlayerBitrate],
                widget.player_bitrate.get().upcast(),
            ),
            (
                &[Setting::AudioBackend],
                widget.audio_backend.get().upcast(),
            ),
            (
                &[Setting::AlsaDevice],
                widget.alsa_device_row.get().upcast(),
            ),
            (&[Setting::AudioFormat], widget.audio_format.get().upcast()),
            (
                &[Setting::GaplessPlayback],
                widget.gapless_playback.get().upcast(),
            ),
            (&[Setting::VolumeCurve], widget.volume_curve.get().upcast()),
            (
                &[Setting::MonoAudio],
                widget.mono_audio_switch.get().upcast(),
            ),
            (
                &[Setting::SkipExplicit],
                widget.skip_explicit_switch.get().upcast(),
            ),
            (
                &[Setting::Normalisation],
                widget.normalisation_switch.get().upcast(),
            ),
            (
                &[Setting::NormalisationType],
                widget.normalisation_type.get().upcast(),
            ),
            (
                &[Setting::NormalisationMethod],
                widget.normalisation_method.get().upcast(),
            ),
            (
                &[Setting::NormalisationPregainDb],
                widget.normalisation_pregain.get().upcast(),
            ),
            (
                &[Setting::NormalisationThresholdDbfs],
                widget.normalisation_threshold.get().upcast(),
            ),
            (
                &[Setting::NormalisationAttackMs],
                widget.normalisation_attack.get().upcast(),
            ),
            (
                &[Setting::NormalisationReleaseMs],
                widget.normalisation_release.get().upcast(),
            ),
            (
                &[Setting::NormalisationKneeDb],
                widget.normalisation_knee.get().upcast(),
            ),
            (&Setting::EQ_BANDS, widget.equalizer.get().upcast()),
            (&[Setting::Pan], widget.pan.get().upcast()),
            (&[Setting::PitchCents], widget.pitch.get().upcast()),
            (&[Setting::ApPort], widget.ap_port_row.get().upcast()),
            (
                &[Setting::AudioCacheSizeMb],
                widget.audio_cache_size.get().upcast(),
            ),
        ]
    }

    /// Offers only what `support` says the player on this computer supports:
    /// other settings are hidden, and choices and ranges narrowed. Stored
    /// values are kept as they are.
    pub fn apply_support(&self, support: SettingsSupport) {
        let widget = self.imp();
        *widget.support.borrow_mut() = support;
        let settings = SettingsStore::new();
        {
            let support = widget.support.borrow();
            for choice in widget.choices.borrow().iter() {
                choice.apply(&support, &settings);
            }
            for number in widget.numbers.borrow().iter() {
                number.apply(&support, &settings);
            }
        }
        self.refresh_visibility();
    }

    /// Shows the rows the player supports, and the groups with a row shown.
    fn refresh_visibility(&self) {
        let widget = self.imp();
        let support = widget.support.borrow();
        let rows = self.player_rows();
        for (settings, row) in &rows {
            row.set_visible(support.supports_any(settings));
        }

        let alsa = widget
            .choices
            .borrow()
            .iter()
            .find(|choice| choice.setting() == Setting::AudioBackend)
            .and_then(ChoiceRow::selected_value)
            == Some("alsa");
        if !alsa {
            widget.alsa_device_row.set_visible(false);
        }

        let has_rows = |group: &libadwaita::PreferencesGroup| {
            rows.iter()
                .any(|(_, row)| row.get_visible() && row.is_ancestor(group))
        };
        widget
            .audio_group
            .set_visible(has_rows(&widget.audio_group));
        widget
            .network_group
            .set_visible(has_rows(&widget.network_group));
        let normalisation = &widget.normalisation_group;
        normalisation.set_visible(
            riff_config::feature_flag::is_enabled(FeatureFlag::Normalisation)
                && has_rows(normalisation),
        );
    }

    /// Updates what's shown when the backend or a feature flag changes.
    fn watch_visibility(&self) {
        let widget = self.imp();
        let dialog = self.downgrade();
        widget.audio_backend.connect_selected_notify(move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.refresh_visibility();
            }
        });

        let store = SettingsStore::new();
        let dialog = self.downgrade();
        store.watch(move |changed| {
            if changed.contains(FeatureFlag::Normalisation.setting()) {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.refresh_visibility();
                }
            }
        });
        let _ = widget.watcher.set(store);
    }

    fn connect_theme_select(&self) {
        let widget = self.imp();
        let theme = widget.theme.downcast_ref::<libadwaita::ComboRow>().unwrap();
        theme.connect_selected_notify(|theme| {
            debug!("Theme switched! --> value: {}", theme.selected());
            let manager = libadwaita::StyleManager::default();

            let pref = match theme.selected() {
                0 => libadwaita::ColorScheme::ForceLight,
                1 => libadwaita::ColorScheme::ForceDark,
                _ => libadwaita::ColorScheme::Default,
            };

            manager.set_color_scheme(pref);
        });
    }

    fn connect_closed<F>(&self, on_closed: F)
    where
        F: Fn() + 'static,
    {
        let dialog = self.upcast_ref::<libadwaita::Dialog>();
        dialog.connect_closed(move |_| on_closed());
    }

    fn connect_close<F>(&self, on_close: F)
    where
        F: Fn() + 'static,
    {
        let dialog = self.upcast_ref::<libadwaita::Dialog>();
        dialog.connect_close_attempt(move |_| {
            on_close();
        });
    }

    /// Connect the "Clear Cache" button to the given handler.
    fn connect_clear_cache<F>(&self, on_clear: F)
    where
        F: Fn() + 'static,
    {
        self.imp()
            .clear_cache_button
            .connect_clicked(move |_| on_clear());
    }

    /// Re-lock the equalizer and pan controls. The locks are UI-only and never
    /// persisted, so they are reset every time the dialog is opened.
    fn reset_locks(&self) {
        self.imp().equalizer.lock();
        self.imp().pan.lock();
        self.imp().pitch.lock();
    }

    /// Apply the Spotify account's explicit content filter lock to the toggle.
    ///
    /// When the account has the filter locked (e.g. a family plan parental
    /// control), the switch is forced on, made insensitive so it cannot be
    /// changed, and given a tooltip explaining why. Otherwise the switch is
    /// interactive and reflects the local preference.
    fn set_explicit_filter_locked(&self, locked: bool) {
        let switch = &self.imp().skip_explicit_switch;
        if locked {
            switch.set_active(true);
            switch.set_sensitive(false);
            switch.set_tooltip_text(Some(&gettextrs::gettext(
                "Locked by a Family plan parental control and cannot be changed here.",
            )));
        } else {
            switch.set_sensitive(true);
            switch.set_tooltip_text(None);
        }
    }
}

pub struct Settings {
    parent: gtk::Window,
    settings_dialog: SettingsDialog,
    model: Rc<SettingsModel>,
}

impl Settings {
    pub fn new(parent: gtk::Window, model: SettingsModel) -> Self {
        let settings_dialog = SettingsDialog::new();
        let model = Rc::new(model);

        let close_model = model.clone();
        settings_dialog.connect_close(move || close_model.apply_settings());

        let gsettings = SettingsStore::new();
        let startup_values: Vec<_> = Setting::restart_required()
            .map(|setting| gsettings.value(setting))
            .collect();
        let restart_model = model.clone();
        settings_dialog.connect_closed(move || {
            let changed = Setting::restart_required()
                .zip(&startup_values)
                .any(|(setting, startup)| gsettings.value(setting) != *startup);
            if changed {
                restart_model.show_restart_required();
            }
        });

        let clear_model = model.clone();
        settings_dialog.connect_clear_cache(move || {
            clear_model.clear_cache();
        });

        Self {
            parent,
            settings_dialog,
            model,
        }
    }

    fn dialog(&self) -> &libadwaita::Dialog {
        self.settings_dialog.upcast_ref::<libadwaita::Dialog>()
    }

    pub fn show_self(&self) {
        self.settings_dialog.reset_locks();
        self.settings_dialog
            .apply_support(self.model.settings_support());
        self.settings_dialog
            .set_explicit_filter_locked(self.model.explicit_filter_locked());
        self.dialog().present(Some(&self.parent));
    }
}

impl EventListener for Settings {
    fn on_event(&mut self, _: &AppEvent) {}
}
