#![allow(mismatched_lifetime_syntaxes)]
#[macro_use(clone)]
extern crate glib;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate gettextrs;

use app::state::{HomePage, Route};
use futures::channel::mpsc::UnboundedSender;
use gettextrs::*;
use gio::prelude::*;
use gio::ApplicationFlags;
use gio::SimpleAction;
use gtk::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

mod app;
mod config;
mod inhibitor;
mod settings;

use crate::app::components::expose_custom_widgets;
use crate::app::dispatch::DispatchLoop;
use crate::app::state::NavigationAction;
use crate::app::{state::PlaybackAction, App, AppAction};

const SEEK_STEP_MS: i32 = 5_000;
const VOLUME_STEP: f64 = 0.05;
#[cfg(target_env = "gnu")]
const MALLOC_ARENAS: libc::c_int = 4;

fn main() {
    #[cfg(target_env = "gnu")]
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, MALLOC_ARENAS);
    }

    riff_playback::player::set_cache_root(glib::user_cache_dir().join("riff").join("librespot"));
    let settings = settings::RiffSettings::new_from_gsettings().unwrap_or_default();
    setup_gtk(&settings);

    expose_custom_widgets();

    let gtk_app = gtk::Application::new(Some(config::APPID), ApplicationFlags::HANDLES_OPEN);
    let builder = gtk::Builder::from_resource("/dev/diegovsky/Riff/window.ui");
    let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();

    if cfg!(debug_assertions) {
        window.add_css_class("devel");
        gtk_app.set_resource_base_path(Some("/dev/diegovsky/Riff"));
    }

    let context = glib::MainContext::default();
    let dispatch_loop = DispatchLoop::new();
    let sender = dispatch_loop.make_dispatcher();

    register_actions(&gtk_app, sender.clone());
    gtk_app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
    gtk_app.set_accels_for_action("win.show-shortcuts", &["<Ctrl>question"]);
    app::components::setup_about(builder.object::<libadwaita::AboutDialog>("about").unwrap());
    setup_arrow_shortcuts(&window, builder.object("arrow_shortcuts").unwrap());

    let app = App::new(settings, builder, sender.clone());
    context.spawn_local(app.attach(dispatch_loop));

    let sender_clone = sender.clone();
    gtk_app.connect_activate(move |gtk_app| {
        debug!("activate");
        if gtk_app.active_window().is_some() {
            sender_clone.unbounded_send(AppAction::Raise).unwrap();
        } else {
            window.set_application(Some(gtk_app));
            gtk_app.add_window(&window);
            sender_clone.unbounded_send(AppAction::Start).unwrap();
        }
    });

    gtk_app.connect_open(move |gtk_app, targets, _| {
        gtk_app.activate();

        let Some(target) = targets.first() else {
            return;
        };
        let uri = target.uri().to_string();
        let action = AppAction::OpenURI(uri)
            .unwrap_or_else(|| AppAction::ShowNotification(gettext("Failed to open link!")));
        sender.unbounded_send(action).unwrap();
    });

    context.invoke_local(move || {
        gtk_app.run();
    });

    if RESTART_REQUESTED.load(Ordering::Relaxed) {
        relaunch();
    }

    std::process::exit(0);
}

static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

fn relaunch() {
    use std::os::unix::process::CommandExt;

    let Ok(exe) = std::env::current_exe() else {
        error!("Cannot restart: unknown executable path");
        return;
    };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec();
    error!("Cannot restart: {err}");
}

fn setup_gtk(settings: &settings::RiffSettings) {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,riff_playback=info,riff_core::remote=info"),
    )
    .init();

    textdomain("riff")
        .and_then(|_| bindtextdomain("riff", config::LOCALEDIR))
        .and_then(|_| bind_textdomain_codeset("riff", "UTF-8"))
        .expect("Could not setup localization");

    gtk::init().unwrap_or_else(|_| panic!("Failed to initialize GTK"));
    libadwaita::init().unwrap_or_else(|_| panic!("Failed to initialize libadwaita"));

    let manager = libadwaita::StyleManager::default();
    manager.set_color_scheme(settings.theme_preference);

    let res = gio::Resource::load(config::PKGDATADIR.to_owned() + "/riff.gresource")
        .expect("Could not load resources");
    gio::resources_register(&res);

    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/dev/diegovsky/Riff/app.css");

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn setup_arrow_shortcuts(
    window: &libadwaita::ApplicationWindow,
    shortcuts: gtk::ShortcutController,
) {
    window.connect_focus_widget_notify(move |window| {
        let typing = GtkWindowExt::focus(window)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>());
        shortcuts.set_propagation_phase(if typing {
            gtk::PropagationPhase::None
        } else {
            gtk::PropagationPhase::Capture
        });
    });
}

fn register_actions(app: &gtk::Application, sender: UnboundedSender<AppAction>) {
    let quit = SimpleAction::new("quit", None);
    quit.connect_activate(clone!(
        #[weak]
        app,
        move |_, _| {
            if let Some(existing_window) = app.active_window() {
                existing_window.close();
            }
            app.quit();
        }
    ));
    app.add_action(&quit);

    let restart = SimpleAction::new("restart", None);
    restart.connect_activate(clone!(
        #[weak]
        app,
        move |_, _| {
            RESTART_REQUESTED.store(true, Ordering::Relaxed);
            app.quit();
        }
    ));
    app.add_action(&restart);

    let resize_window =
        SimpleAction::new("resize_window", Some(glib::VariantTy::new("(ii)").unwrap()));
    resize_window.connect_activate(clone!(
        #[weak]
        app,
        move |_, parameter| {
            let Some((width, height)) = parameter.and_then(|p| p.get::<(i32, i32)>()) else {
                return;
            };
            if let Some(window) = app.active_window() {
                window.unmaximize();
                window.set_default_size(width, height);
            }
        }
    ));
    app.add_action(&resize_window);

    app.add_action(&make_action(
        "toggle_playback",
        PlaybackAction::TogglePlay.into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "player_prev",
        PlaybackAction::Previous.into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "player_next",
        PlaybackAction::Next.into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "toggle_shuffle",
        PlaybackAction::ToggleShuffle.into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "toggle_repeat",
        PlaybackAction::ToggleRepeat.into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "seek_backward",
        PlaybackAction::SeekBy(-SEEK_STEP_MS).into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "seek_forward",
        PlaybackAction::SeekBy(SEEK_STEP_MS).into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "volume_down",
        PlaybackAction::AdjustVolume(-VOLUME_STEP).into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "volume_up",
        PlaybackAction::AdjustVolume(VOLUME_STEP).into(),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "toggle_mute",
        PlaybackAction::ToggleMute.into(),
        sender.clone(),
    ));

    for (name, page) in [
        ("nav_now_playing", HomePage::NowPlaying),
        ("nav_artists", HomePage::SavedArtists),
        ("nav_albums", HomePage::Library),
        ("nav_playlists", HomePage::SavedPlaylists),
        ("nav_tracks", HomePage::SavedTracks),
    ] {
        app.add_action(&make_action(
            name,
            NavigationAction::Go(Route::Home(page)).into(),
            sender.clone(),
        ));
    }

    app.add_action(&make_action(
        "nav_back",
        AppAction::from(NavigationAction::Back),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "nav_forward",
        AppAction::from(NavigationAction::Forward),
        sender.clone(),
    ));

    app.add_action(&make_action(
        "search",
        AppAction::from(NavigationAction::Go(Route::Search)),
        sender.clone(),
    ));
}

fn make_action(
    name: &str,
    app_action: AppAction,
    sender: UnboundedSender<AppAction>,
) -> SimpleAction {
    let action = SimpleAction::new(name, None);
    action.connect_activate(move |_, _| {
        sender.unbounded_send(app_action.clone()).unwrap();
    });
    action
}
