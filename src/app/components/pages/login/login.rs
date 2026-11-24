use gettextrs::gettext;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use url::Url;

use crate::app::components::engine_setup::playback_ready;
use crate::app::components::EventListener;
use crate::app::state::{SessionChanged, SessionFields, SessionStatus};
use crate::app::AppEvent;
use riff_core::events::EngineStatus;
use riff_playback::player::soloist::looks_like_key;
use riff_playback::player::Engine;

use super::spinning_logo::SpinningLogo;
use super::LoginModel;
mod imp {

    use libadwaita::subclass::prelude::AdwWindowImpl;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/dev/diegovsky/Riff/components/login.ui")]
    pub struct LoginWindow {
        #[template_child]
        pub layout: TemplateChild<gtk::Box>,

        #[template_child]
        pub pages: TemplateChild<gtk::Stack>,

        #[template_child]
        pub loading_slot: TemplateChild<gtk::Box>,

        #[template_child]
        pub welcome_logo_slot: TemplateChild<gtk::Box>,

        pub logo: SpinningLogo,

        #[template_child]
        pub welcome_content: TemplateChild<gtk::Box>,

        #[template_child]
        pub login_with_librespot_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub login_with_soloist_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub toast_overlay: TemplateChild<libadwaita::ToastOverlay>,

        #[template_child]
        pub key_body: TemplateChild<gtk::Label>,

        #[template_child]
        pub key_entry: TemplateChild<gtk::PasswordEntry>,

        #[template_child]
        pub key_continue_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub setup_logo: TemplateChild<SpinningLogo>,

        #[template_child]
        pub setup_problem_icon: TemplateChild<gtk::Image>,

        #[template_child]
        pub setup_title: TemplateChild<gtk::Label>,

        #[template_child]
        pub setup_guide: TemplateChild<gtk::Box>,

        #[template_child]
        pub setup_guide_desktop: TemplateChild<gtk::Picture>,

        #[template_child]
        pub setup_guide_mobile: TemplateChild<gtk::Picture>,

        #[template_child]
        pub setup_guide_desktop_expand: TemplateChild<gtk::Button>,

        #[template_child]
        pub setup_guide_mobile_expand: TemplateChild<gtk::Button>,

        #[template_child]
        pub setup_body: TemplateChild<gtk::Label>,

        #[template_child]
        pub setup_report_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub header: TemplateChild<libadwaita::HeaderBar>,

        #[template_child]
        pub back_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub about_button: TemplateChild<gtk::Button>,

        #[template_child]
        pub auth_continue_button: TemplateChild<gtk::Button>,

        pub auth_url: RefCell<Option<Url>>,
        pub open_when_ready: std::cell::Cell<bool>,
        pub setup_back: std::cell::Cell<bool>,
        pub problem: RefCell<Option<libadwaita::Toast>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LoginWindow {
        const NAME: &'static str = "LoginWindow";
        type Type = super::LoginWindow;
        type ParentType = libadwaita::Window;

        fn class_init(klass: &mut Self::Class) {
            SpinningLogo::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LoginWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.loading_slot.append(&self.logo);
            self.logo.spin();
            self.setup_logo.spin();
            super::play_guide(&self.setup_guide_desktop, "desktop-soloist-setup.webm");
            super::play_guide(&self.setup_guide_mobile, "mobile-soloist-setup.webm");
            let obj = self.obj();
            obj.connect_guide_expand(
                &self.setup_guide_desktop,
                &self.setup_guide_desktop_expand,
                gettext("Desktop"),
            );
            obj.connect_guide_expand(
                &self.setup_guide_mobile,
                &self.setup_guide_mobile_expand,
                gettext("Mobile"),
            );
            let window = self.obj().downgrade();
            self.pages.connect_visible_child_name_notify(move |pages| {
                if let Some(window) = window.upgrade() {
                    window.update_back_button();
                }
                if pages.visible_child_name().as_deref() != Some("setup") {
                    if let Some(window) = window.upgrade() {
                        window.imp().setup_guide.set_visible(false);
                    }
                }
            });
            self.obj().update_back_button();
        }
    }
    impl WidgetImpl for LoginWindow {}
    impl AdwWindowImpl for LoginWindow {}
    impl WindowImpl for LoginWindow {}
}

glib::wrapper! {
    pub struct LoginWindow(ObjectSubclass<imp::LoginWindow>) @extends gtk::Widget, gtk::Window, libadwaita::Window,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Default for LoginWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthProblem {
    NotPremium,
    LoginFailed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StepIcon {
    Waiting,
    Problem,
    None,
}

pub(super) struct Step {
    title: String,
    body: String,
    icon: StepIcon,
    back: bool,
    guide: bool,
    report: bool,
}

impl LoginWindow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn connect_close<F>(&self, on_close: F)
    where
        F: Fn() + 'static,
    {
        let window = self.upcast_ref::<libadwaita::Window>();
        window.connect_close_request(move |_| {
            on_close();
            glib::Propagation::Proceed
        });
    }

    fn connect_login<F>(&self, on_login: F)
    where
        F: Fn(Engine) + 'static,
    {
        let on_login = Rc::new(on_login);
        let librespot = Rc::clone(&on_login);
        self.imp()
            .login_with_librespot_button
            .connect_clicked(move |_| librespot(Engine::Librespot));
        self.imp()
            .login_with_soloist_button
            .connect_clicked(move |_| on_login(Engine::Soloist));
    }

    fn connect_key<K, B>(&self, on_key: K, on_back: B)
    where
        K: Fn(String) + 'static,
        B: Fn() + 'static,
    {
        let imp = self.imp();
        let button = imp.key_continue_button.get();
        let warned = std::cell::Cell::new(false);
        imp.key_entry.connect_changed(clone!(
            #[weak(rename_to = window)]
            self,
            move |entry| {
                let text = entry.text();
                let looks_right = looks_like_key(&text);
                button.set_sensitive(looks_right);
                let wrong = !text.trim().is_empty() && !looks_right;
                if wrong && !warned.replace(true) {
                    window.show_problem(&gettext(
                        "That doesn’t look like an API key. Copy it with the copy button next to the key",
                    ));
                } else if !wrong && warned.replace(false) {
                    window.reset_status();
                }
            }
        ));
        let on_key = Rc::new(on_key);
        let entry = imp.key_entry.get();
        let on_continue = Rc::clone(&on_key);
        imp.key_continue_button
            .connect_clicked(move |_| on_continue(entry.text().trim().to_string()));
        imp.key_entry.connect_activate(move |entry| {
            let key = entry.text().trim().to_string();
            if looks_like_key(&key) {
                on_key(key);
            }
        });
        self.connect_back("key", on_back);
    }

    fn connect_back<F>(&self, page: &'static str, on_back: F)
    where
        F: Fn() + 'static,
    {
        let pages = self.imp().pages.get();
        self.imp().back_button.connect_clicked(move |_| {
            if pages.visible_child_name().as_deref() == Some(page) {
                on_back();
            }
        });
    }

    fn connect_setup<F>(&self, on_back: F)
    where
        F: Fn() + 'static,
    {
        self.imp()
            .setup_report_button
            .connect_clicked(|_| crate::app::components::report_issue());
        self.connect_back("setup", on_back);
    }

    fn update_back_button(&self) {
        let imp = self.imp();
        let page = imp.pages.visible_child_name();
        let shown = match page.as_deref() {
            Some("auth" | "key") => true,
            Some("setup") => imp.setup_back.get(),
            _ => false,
        };
        imp.back_button.set_visible(shown);
        imp.about_button
            .set_visible(page.as_deref() == Some("choose"));
    }

    fn connect_guide_expand(&self, picture: &gtk::Picture, button: &gtk::Button, title: String) {
        if picture.paintable().is_none() {
            button.set_visible(false);
            return;
        }
        picture.set_cursor_from_name(Some("zoom-in"));
        let title = Rc::new(title);
        let click = gtk::GestureClick::new();
        click.connect_released(clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            picture,
            #[strong]
            title,
            move |_, _, _, _| window.expand_guide(&picture, &title)
        ));
        picture.add_controller(click);
        button.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            picture,
            move |_| window.expand_guide(&picture, &title)
        ));
    }

    fn expand_guide(&self, picture: &gtk::Picture, title: &str) {
        let Some(recording) = picture.paintable() else {
            return;
        };
        let larger = gtk::Picture::builder()
            .paintable(&recording)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        let view = libadwaita::ToolbarView::new();
        view.add_top_bar(&libadwaita::HeaderBar::new());
        view.set_content(Some(&larger));
        let dialog = libadwaita::Dialog::builder()
            .title(title)
            .content_width(960)
            .content_height(700)
            .child(&view)
            .build();
        libadwaita::prelude::AdwDialogExt::present(&dialog, Some(self));
    }

    fn connect_about(&self, about: libadwaita::AboutDialog) {
        self.imp().about_button.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| libadwaita::prelude::AdwDialogExt::present(&about, Some(&window))
        ));
    }

    fn connect_auth<R, B>(&self, on_retry: R, on_back: B)
    where
        R: Fn() + 'static,
        B: Fn() + 'static,
    {
        let imp = self.imp();
        imp.auth_continue_button.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let imp = window.imp();
                match imp.auth_url.borrow().as_ref() {
                    Some(url) => open_in_browser(url),
                    None => {
                        imp.open_when_ready.set(true);
                        on_retry();
                    }
                }
            }
        ));
        self.connect_back("auth", on_back);
    }

    pub(super) fn show_auth_page(&self, url: Url) {
        let imp = self.imp();
        self.reset_status();
        if imp.open_when_ready.take() {
            open_in_browser(&url);
        }
        imp.auth_url.replace(Some(url));
        imp.pages.set_visible_child_name("auth");
        imp.auth_continue_button.grab_focus();
    }

    pub(super) fn show_auth_problem(&self, problem: AuthProblem) {
        let imp = self.imp();
        imp.auth_url.replace(None);
        imp.open_when_ready.set(false);
        imp.pages.set_visible_child_name("auth");
        let text = match problem {
            AuthProblem::NotPremium => {
                // Translators: Shown when a non-premium account is detected.
                gettext("Your account does not have a Spotify Premium subscription")
            }
            AuthProblem::LoginFailed => {
                // Translators: Shown when login fails. Browser extensions such
                // as ad or privacy blockers can break the login page.
                gettext("Login failed. Please check your connection and browser extensions before trying again")
            }
        };
        self.show_problem(&text);
        imp.auth_continue_button.grab_focus();
    }

    pub(super) fn show_loading(&self) {
        let imp = self.imp();
        move_to(&imp.logo, &imp.loading_slot);
        imp.logo.spin();
        imp.pages.set_visible_child_name("loading");
    }

    pub(super) fn show_choices(&self) {
        let imp = self.imp();
        imp.key_entry.set_text("");
        let pages = &imp.pages;
        if pages.visible_child_name().as_deref() == Some("loading") && pages.is_mapped() {
            pages.add_tick_callback(clone!(
                #[weak(rename_to = window)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move |pages, _| {
                    if pages.visible_child_name().as_deref() == Some("loading") {
                        window.hand_logo_to_welcome();
                    }
                    glib::ControlFlow::Break
                }
            ));
        } else {
            self.settle_on_choices();
        }
    }

    pub(super) fn settle_on_choices(&self) {
        let imp = self.imp();
        imp.key_entry.set_text("");
        move_to(&imp.logo, &imp.welcome_logo_slot);
        imp.logo.rest();
        imp.pages
            .set_visible_child_full("choose", gtk::StackTransitionType::None);
    }

    fn hand_logo_to_welcome(&self) {
        let imp = self.imp();
        let logo = &imp.logo;
        let from = logo.window_y().filter(|_| logo.width() > 0);
        move_to(logo, &imp.welcome_logo_slot);
        if let Some(from) = from {
            logo.glide_from(from);
        }
        logo.settle();
        imp.pages
            .set_visible_child_full("choose", gtk::StackTransitionType::None);
        slide_in(&imp.welcome_content);
    }

    pub(super) fn show_key_page(&self, key_page: &str, refused: bool) {
        let link = glib::markup_escape_text(key_page);

        let body = gettext(
            "Soloist requires an API key from your account. \
             <a href=\"{}\">Please create a key</a> and paste it below.",
        )
        .replace("{}", &link);
        let imp = self.imp();
        imp.key_body.set_markup(&body);
        imp.key_entry.set_text("");
        self.reset_status();
        imp.pages.set_visible_child_name("key");
        imp.key_entry.grab_focus();
        if refused {
            self.show_problem(&gettext(
                "That key didn’t work. Check that you copied the whole key, or create a new one",
            ));
        }
    }

    #[cfg(debug_assertions)]
    pub fn header(&self) -> libadwaita::HeaderBar {
        self.imp().header.get()
    }

    pub(super) fn show_step(&self, step: Step) {
        let imp = self.imp();
        imp.setup_title.set_label(&step.title);
        imp.setup_body.set_label(&step.body);
        imp.setup_body.set_visible(!step.body.is_empty());
        imp.setup_report_button.set_visible(step.report);
        imp.setup_back.set(step.back);
        imp.setup_guide.set_visible(step.guide);
        imp.setup_logo.set_visible(step.icon == StepIcon::Waiting);
        imp.setup_problem_icon
            .set_visible(step.icon == StepIcon::Problem);
        imp.pages.set_visible_child_name("setup");
        self.update_back_button();
    }

    pub(super) fn show_problem(&self, text: &str) {
        self.reset_status();
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(40)
            .xalign(0.0)
            .build();
        let toast = libadwaita::Toast::builder()
            .custom_title(&label)
            .timeout(0)
            .priority(libadwaita::ToastPriority::High)
            .build();
        self.imp().toast_overlay.add_toast(toast.clone());
        self.imp().problem.replace(Some(toast));
    }
    pub(super) fn reset_status(&self) {
        if let Some(toast) = self.imp().problem.take() {
            toast.dismiss();
        }
    }
}

const GUIDE_RESOURCES: &str = "/dev/diegovsky/Riff/guide";

fn play_guide(picture: &gtk::Picture, name: &str) {
    let path = format!("{GUIDE_RESOURCES}/{name}");
    if gio::resources_get_info(&path, gio::ResourceLookupFlags::NONE).is_err() {
        debug!("There's no {path} yet; its guide stays empty");
        return;
    }
    let media = gtk::MediaFile::for_resource(&path);
    media.set_loop(false);
    media.set_muted(true);
    picture.set_paintable(Some(&media));
    media.connect_ended_notify(clone!(
        #[weak]
        picture,
        move |media| {
            if !media.is_ended() {
                return;
            }
            media.set_resource(Some(&path));
            if picture.is_mapped() && picture.settings().is_gtk_enable_animations() {
                media.play();
            }
        }
    ));
    picture.connect_map(clone!(
        #[weak]
        media,
        move |picture| {
            if picture.settings().is_gtk_enable_animations() {
                media.play();
            }
        }
    ));
    picture.connect_unmap(clone!(
        #[weak]
        media,
        move |_| media.pause()
    ));
}

fn open_in_browser(url: &Url) {
    if open::that(url.as_str()).is_err() {
        warn!("Could not open login page");
    }
}
fn move_to(logo: &SpinningLogo, slot: &gtk::Box) {
    if logo.parent().as_ref() == Some(slot.upcast_ref()) {
        return;
    }
    if let Some(parent) = logo.parent().and_downcast::<gtk::Box>() {
        parent.remove(logo);
    }
    slot.append(logo);
}

const WELCOME_ENTERING: &str = "welcome--entering";
const WELCOME_SLIDE: std::time::Duration = std::time::Duration::from_millis(800 + 160);

fn slide_in(content: &gtk::Box) {
    content.remove_css_class(WELCOME_ENTERING);
    content.add_css_class(WELCOME_ENTERING);
    glib::timeout_add_local_once(
        WELCOME_SLIDE,
        clone!(
            #[weak]
            content,
            move || content.remove_css_class(WELCOME_ENTERING)
        ),
    );
}

pub(super) fn step_for(engine: &EngineStatus) -> Option<Step> {
    let waiting = |title: String, body: String| Step {
        title,
        body,
        icon: StepIcon::Waiting,
        back: false,
        guide: false,
        report: false,
    };
    Some(match engine {
        EngineStatus::Ready | EngineStatus::NeedsKey { .. } | EngineStatus::BuiltIn => return None,
        EngineStatus::Installing => Step {
            // Translators: Shown while the player is downloaded. Soloist is
            // its name and isn't translated.
            title: gettext("Downloading Soloist"),
            body: String::new(),
            icon: StepIcon::Waiting,
            back: false,
            guide: false,
            report: false,
        },
        EngineStatus::Updating => Step {
            // Translators: Shown while a newer player is downloaded for one
            // that stopped working. Soloist is its name and isn't translated.
            title: gettext("Updating Soloist"),
            body: String::new(),
            icon: StepIcon::Waiting,
            back: false,
            guide: false,
            report: false,
        },
        EngineStatus::NeedsPairing { device_name } => Step {
            icon: StepIcon::None,
            back: true,
            guide: true,
            ..waiting(
                // Translators: Shown while the player waits to be picked as a
                // device in another app, which signs it in.
                gettext("Setup Soloist"),
                // Translators: {} is the name this computer shows up under
                // among devices.
                gettext(
                    "In the official app on your phone or computer, choose “{}” as the device to play on",
                )
                .replace("{}", device_name),
            )
        },
        EngineStatus::Failed => Step {
            // Translators: Soloist is the player's name and isn't translated
            title: gettext("Soloist Isn’t Available"),
            body: soloist_failed_text(),
            icon: StepIcon::Problem,
            back: true,
            guide: false,
            report: true,
        },
    })
}

pub fn soloist_failed_text() -> String {
    // Translators: Soloist is the player's name and isn't translated
    gettext("Soloist couldn’t be downloaded or set up on this computer.")
}

pub fn soloist_updating_text() -> String {
    // Translators: Soloist is the player's name and isn't translated
    gettext("Soloist needs an update. Downloading it now…")
}

pub struct Login {
    parent: gtk::Window,
    login_window: LoginWindow,
    model: Rc<LoginModel>,
    set_up: Cell<bool>,
    told: Cell<bool>,
}

impl Login {
    pub fn new(parent: gtk::Window, model: LoginModel, about: libadwaita::AboutDialog) -> Self {
        let model = Rc::new(model);

        let login_window = LoginWindow::new();
        login_window.connect_about(about);

        login_window.connect_close(clone!(
            #[weak]
            parent,
            move || {
                if let Some(app) = parent.application() {
                    app.quit();
                }
            }
        ));

        login_window.connect_login(clone!(
            #[weak]
            model,
            #[weak]
            login_window,
            move |engine| {
                login_window.reset_status();
                model.log_in_with(engine);
            }
        ));

        login_window.connect_key(
            clone!(
                #[weak]
                model,
                move |key| model.set_key(key)
            ),
            clone!(
                #[weak]
                model,
                move || model.cancel()
            ),
        );

        login_window.connect_auth(
            clone!(
                #[weak]
                model,
                move || model.retry_login()
            ),
            clone!(
                #[weak]
                model,
                move || model.cancel()
            ),
        );

        login_window.connect_setup(clone!(
            #[weak]
            model,
            move || model.cancel()
        ));

        Self {
            parent,
            login_window,
            model,
            set_up: Cell::new(false),
            told: Cell::new(false),
        }
    }

    #[cfg(debug_assertions)]
    pub fn login_window(&self) -> LoginWindow {
        self.login_window.clone()
    }

    fn window(&self) -> &libadwaita::Window {
        self.login_window.upcast_ref::<libadwaita::Window>()
    }

    fn show_self(&self) {
        self.window().set_transient_for(Some(&self.parent));
        self.window().set_modal(true);
        self.window().set_visible(true);
    }

    fn hide(&self) {
        self.login_window.reset_status();
        self.login_window.settle_on_choices();
        self.window().set_visible(false);
    }

    fn reveal_problem(&self, problem: AuthProblem) {
        self.show_self();
        self.login_window.show_auth_problem(problem);
    }

    fn open_login_url(&self, url: Url) {
        self.show_self();
        self.login_window.show_auth_page(url);
    }

    fn logged_in(&self, session: &SessionChanged) {
        if playback_ready(session) {
            self.set_up.set(true);
            self.told.set(false);
            self.hide();
            return;
        }
        if self.set_up.get() {
            self.changed_while_open(&session.engine);
            return;
        }
        self.show_engine_step(&session.engine);
    }

    fn changed_while_open(&self, engine: &EngineStatus) {
        match engine {
            EngineStatus::NeedsKey { .. } | EngineStatus::NeedsPairing { .. } => {
                self.show_engine_step(engine);
            }
            EngineStatus::Updating if !self.told.get() => {
                self.told.set(true);
                self.model.notify(soloist_updating_text());
            }
            EngineStatus::Failed => self.model.notify(soloist_failed_text()),
            _ => {}
        }
    }

    fn show_engine_step(&self, engine: &EngineStatus) {
        match engine {
            EngineStatus::NeedsKey { key_page, refused } => {
                self.show_self();
                self.login_window.show_key_page(key_page, *refused);
            }
            EngineStatus::BuiltIn => {
                self.show_self();
                self.login_window.show_loading();
            }
            engine => {
                if let Some(step) = step_for(engine) {
                    self.show_self();
                    self.login_window.show_step(step);
                }
            }
        }
    }
}

impl EventListener for Login {
    fn on_event(&mut self, event: &AppEvent) {
        if let AppEvent::Started = event {
            self.show_self();
            self.login_window.show_loading();
            self.model.try_autologin();
        }
        if let AppEvent::Raised = event {
            if self.window().is_visible() {
                self.window().present();
            }
            return;
        }
        let Some(session) = event.session().filter(|s| {
            s.changed
                .intersects(SessionFields::STATUS | SessionFields::ENGINE)
        }) else {
            return;
        };
        match &session.status {
            SessionStatus::LoggedIn => self.logged_in(session),
            SessionStatus::NotPremium => self.reveal_problem(AuthProblem::NotPremium),
            SessionStatus::Failed => self.reveal_problem(AuthProblem::LoginFailed),
            SessionStatus::WaitingForBrowser(url) => {
                if session.changed.contains(SessionFields::STATUS) {
                    self.open_login_url(url.clone())
                }
            }
            SessionStatus::LoggedOut => {
                self.set_up.set(false);
                if session.changed.contains(SessionFields::STATUS) {
                    self.show_self();
                    self.login_window.show_choices();
                }
            }
            SessionStatus::LoggingIn(_) => {}
        }
    }
}
