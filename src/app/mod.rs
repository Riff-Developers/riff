use crate::app::state::Action;
use crate::settings::{RiffSettings, StateTracker};
use futures::channel::mpsc::UnboundedSender;
use glib::object::Cast;
use riff_auth::{SessionTokens, TokenStore};
use riff_config::setting::megabytes;
use riff_config::{Setting, SettingsChanged, SettingsStore};
use riff_core::schedule::RunLog;
#[cfg(debug_assertions)]
use riff_playback::player::Command;
use riff_playback::player::{ThisComputer, ThisDevice};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub mod dispatch;
pub use dispatch::{DispatchLoop, Dispatcher};

pub mod components;
use components::*;

pub mod models;
pub use models::SongsSource;

mod list_store;
pub use list_store::*;

pub mod state;
pub use state::{AppAction, AppEvent, AppModel, AppState, ProvidesApi};

mod mpris;
mod pin_store;
pub mod player_events;
mod remote_playback;
use pin_store::PinStore;

#[cfg(debug_assertions)]
mod dev_tools;

pub struct App {
    settings: RiffSettings,
    builder: gtk::Builder,
    components: Vec<Box<dyn EventListener>>,
    model: Rc<AppModel>,
    sender: UnboundedSender<AppAction>,
    this_computer: ThisComputer,
    // Sends commands to the librespot player. Only the dev tools menu uses it,
    // so it is compiled out of release builds entirely.
    #[cfg(debug_assertions)]
    player_command_sender: UnboundedSender<Command>,
    _settings: SettingsStore,
}

impl App {
    pub fn new(
        settings: RiffSettings,
        builder: gtk::Builder,
        sender: UnboundedSender<AppAction>,
    ) -> Self {
        let state = AppState::default();

        let token_store = TokenStore::new();
        let token_provider: Arc<dyn riff_api::TokenProvider> = Arc::new(token_store.clone());
        let session_tokens = SessionTokens::new();
        let session_token_provider: Arc<dyn riff_api::TokenProvider> =
            Arc::new(session_tokens.clone());

        let settings_store = SettingsStore::new();

        let api_service = Arc::new(riff_api::spotify_service(
            token_provider,
            session_token_provider,
            megabytes(settings_store.uint(Setting::MemoryCacheSizeMb)),
            megabytes(settings_store.uint(Setting::DiskCacheSizeMb)),
        ));

        let dispatcher = Dispatcher::new(sender.clone());
        settings_store.watch(move |changed| {
            dispatcher.dispatch(AppAction::Core(Action::SettingsChanged(changed)));
        });
        let model = Rc::new(AppModel::new(state, api_service));

        let player_events = player_events::forward_player_events(sender.clone());
        let effects = {
            let player = &settings.player_settings;
            riff_audio_engine::Effects::new(
                player.eq_bands,
                player.mono_audio,
                player.pan,
                player.pitch_cents,
            )
        };
        let player_command_sender = riff_playback::player::start_player_service(
            settings.player_settings.clone(),
            player_events.clone(),
            token_store,
            session_tokens,
            effects.clone(),
            model.api().uris(),
        );
        let run_log = RunLog::open(glib::user_data_dir().join("riff").join("schedule"));
        let this_computer = ThisComputer::new(
            ThisDevice::new(player_command_sender.clone()),
            remote_playback::start_soloist(&model.api(), player_events.clone(), effects, run_log),
        );
        let engine_setup = EngineSetup::new(this_computer.clone());

        let controller = remote_playback::start_remote_playback(
            model.api(),
            Arc::new(this_computer.clone()),
            sender.clone(),
        );

        let components: Vec<Box<dyn EventListener>> = vec![
            Box::new(PlayerNotifier::new(
                Dispatcher::new(sender.clone()),
                player_command_sender.clone(),
                controller,
            )),
            Box::new(StateTracker::new_from_gsettings()),
            Box::new(CacheSizes {
                api: model.api(),
                settings: settings_store.clone(),
            }),
            Box::new(PinStore::new(
                Rc::clone(&model),
                Dispatcher::new(sender.clone()),
            )),
            App::make_mpris(Rc::clone(&model), sender.clone()),
            App::make_inhibitor(&builder, Rc::clone(&model)),
        ];
        let components = components
            .into_iter()
            .chain([Box::new(engine_setup) as Box<dyn EventListener>])
            .collect();

        Self {
            settings,
            builder,
            components,
            model,
            sender,
            this_computer,
            #[cfg(debug_assertions)]
            player_command_sender,
            _settings: settings_store,
        }
    }

    fn add_ui_components(&mut self) {
        let builder = &self.builder;
        let model = &self.model;
        let sender = &self.sender;
        let dispatcher = Dispatcher::new(sender.clone());

        for action in self.settings.player_settings.actions() {
            sender.unbounded_send(action.into()).unwrap();
        }

        let app_header: AppHeaderBar = builder.object("app_header").unwrap();
        let header_models: HeaderModelRegistry = Rc::new(RefCell::new(HashMap::new()));
        let registrar = HeaderRegistrar::new(app_header.clone(), Rc::clone(&header_models));

        let layout = WindowLayout::new(builder);

        let login = App::make_login(builder, dispatcher.clone(), self.this_computer.clone());
        #[cfg(debug_assertions)]
        let login_window = login.login_window();

        let mut components: Vec<Box<dyn EventListener>> = vec![
            App::make_window(&self.settings, builder, Rc::clone(model)),
            App::make_selection_toolbar(builder, Rc::clone(model), dispatcher.clone()),
            App::make_playback(builder, Rc::clone(model), dispatcher.clone()),
            App::make_device_selector(builder, Rc::clone(model), dispatcher.clone()),
            App::make_panel_manager(
                builder,
                layout,
                app_header.clone(),
                Rc::clone(model),
                dispatcher.clone(),
            ),
            login,
            App::make_navigation(builder, Rc::clone(model), dispatcher.clone(), registrar),
            App::make_app_header(
                app_header,
                Rc::clone(model),
                dispatcher.clone(),
                header_models,
            ),
            App::make_search_button(builder, dispatcher.clone()),
            App::make_clipboard_import(builder, Rc::clone(model), dispatcher.clone()),
            App::make_user_menu(builder, Rc::clone(model), dispatcher),
            App::make_notification(builder),
        ];

        // Wire up dev tools menu (debug builds only)
        #[cfg(debug_assertions)]
        dev_tools::wire_dev_tools(
            builder,
            sender,
            &self.player_command_sender,
            &self.this_computer,
            model,
            &login_window,
        );

        self.components.append(&mut components);
    }

    fn make_mpris(
        app_model: Rc<AppModel>,
        sender: UnboundedSender<AppAction>,
    ) -> Box<impl EventListener> {
        Box::new(mpris::MprisListener::start(app_model, sender))
    }

    fn make_window(
        settings: &RiffSettings,
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
    ) -> Box<impl EventListener> {
        let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();
        Box::new(MainWindow::new(settings.window.clone(), app_model, window))
    }

    fn make_app_header(
        widget: AppHeaderBar,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
        models: HeaderModelRegistry,
    ) -> Box<AppHeaderBarComponent> {
        Box::new(AppHeaderBarComponent::new(
            widget, app_model, dispatcher, models,
        ))
    }

    fn make_inhibitor(builder: &gtk::Builder, app_model: Rc<AppModel>) -> Box<impl EventListener> {
        let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();
        Box::new(crate::inhibitor::SuspendInhibitor::new(window, app_model))
    }

    fn make_navigation(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
        registrar: HeaderRegistrar,
    ) -> Box<Navigation> {
        let split_view: libadwaita::OverlaySplitView = builder.object("navigation_split").unwrap();
        let content_stack: gtk::Stack = builder.object("content_stack").unwrap();
        let home_listbox: gtk::ListBox = builder.object("home_listbox").unwrap();
        let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();

        let model = NavigationModel::new(Rc::clone(&app_model), dispatcher.clone());
        let screen_factory =
            ScreenFactory::new(Rc::clone(&app_model), dispatcher.clone(), registrar);
        Box::new(Navigation::new(
            model,
            split_view,
            content_stack,
            home_listbox,
            screen_factory,
            window,
        ))
    }

    fn make_login(
        builder: &gtk::Builder,
        dispatcher: Dispatcher,
        this_computer: ThisComputer,
    ) -> Box<Login> {
        let parent: gtk::Window = builder.object("window").unwrap();
        let about: libadwaita::AboutDialog = builder.object("about").unwrap();
        let model = LoginModel::new(dispatcher, this_computer);
        Box::new(Login::new(parent, model, about))
    }

    fn make_selection_toolbar(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<impl EventListener> {
        Box::new(SelectionToolbar::new(
            SelectionToolbarModel::new(app_model, dispatcher),
            builder.object("selection_toolbar").unwrap(),
        ))
    }

    fn make_playback(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<impl EventListener> {
        let model = PlaybackModel::new(app_model, dispatcher);
        Box::new(PlaybackControl::new(
            model,
            builder.object("playback").unwrap(),
            builder.object("queue_bar").unwrap(),
        ))
    }

    fn make_device_selector(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<impl EventListener> {
        let playback: PlaybackWidget = builder.object("playback").unwrap();
        Box::new(DeviceSelector::new(
            playback.device_selector(),
            DeviceSelectorModel::new(app_model, dispatcher),
        ))
    }

    fn make_panel_manager(
        builder: &gtk::Builder,
        layout: Rc<WindowLayout>,
        app_header: AppHeaderBar,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<PanelManager> {
        let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();
        let panel = UtilityPanel::new(
            builder.object("utility_panel").unwrap(),
            builder.object("utility_split").unwrap(),
            layout,
        );
        let has_track = app_model.get_state().playback.current_song().is_some();
        let sheet = BottomSheet::new(
            builder.object("bottom_sheet").unwrap(),
            builder.object("queue_bar").unwrap(),
            has_track,
        );
        Box::new(PanelManager::new(
            &window, panel, sheet, app_header, app_model, dispatcher,
        ))
    }

    fn make_search_button(builder: &gtk::Builder, dispatcher: Dispatcher) -> Box<SearchButton> {
        let search_button: gtk::Button = builder.object("search_button").unwrap();
        let model = SearchBarModel(dispatcher);
        Box::new(SearchButton::new(model, search_button))
    }

    fn make_user_menu(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<UserMenu> {
        let parent: libadwaita::ApplicationWindow = builder.object("window").unwrap();
        let settings_model = SettingsModel::new(app_model.clone(), dispatcher.clone());
        let settings = Settings::new(parent.clone().upcast(), settings_model);

        let button: gtk::MenuButton = builder.object("user").unwrap();
        let main_menu: gio::Menu = builder.object("main_menu").unwrap();
        let about: libadwaita::AboutDialog = builder.object("about").unwrap();
        let shortcuts_dialog = make_shortcuts_dialog();
        let model = UserMenuModel::new(app_model, dispatcher);
        let user_menu = UserMenu::new(
            button,
            main_menu,
            settings,
            about,
            shortcuts_dialog,
            parent,
            model,
        );
        Box::new(user_menu)
    }

    fn make_notification(builder: &gtk::Builder) -> Box<Notification> {
        let toast_overlay: libadwaita::ToastOverlay = builder.object("toast_overlay").unwrap();
        Box::new(Notification::new(toast_overlay))
    }

    fn make_clipboard_import(
        builder: &gtk::Builder,
        app_model: Rc<AppModel>,
        dispatcher: Dispatcher,
    ) -> Box<ClipboardImport> {
        let window: libadwaita::ApplicationWindow = builder.object("window").unwrap();
        Box::new(ClipboardImport::new(window, dispatcher, app_model))
    }

    fn handle(&mut self, action: AppAction) {
        let starting = matches!(&action, &AppAction::Start);

        let events = self.model.update_state(action);

        if !events.is_empty() && starting {
            self.add_ui_components();
        }

        for event in events.iter() {
            info!("Event: {event:?}");
            for component in self.components.iter_mut() {
                component.on_event(event);
            }
        }
    }

    pub async fn attach(mut self, dispatch_loop: DispatchLoop) {
        let rt = Box::leak(Box::new(
            tokio::runtime::Runtime::new().expect("Failed to acquire tokio runtime"),
        ));
        std::mem::forget(rt.enter());

        let maintenance = self.model.api();
        rt.spawn(async move {
            maintenance.run_cache_maintenance().await;
        });

        let app = &mut self;
        dispatch_loop
            .attach(move |action| {
                info!("Action: {action:?}");
                app.handle(action);
            })
            .await;
    }
}

struct CacheSizes {
    api: Arc<riff_api::ApiService>,
    settings: SettingsStore,
}

impl CacheSizes {
    fn apply(&self, changed: &SettingsChanged) {
        if changed.contains(Setting::MemoryCacheSizeMb) {
            let bytes = megabytes(self.settings.uint(Setting::MemoryCacheSizeMb));
            self.api.set_memory_cache_bytes(bytes);
        }
        if changed.contains(Setting::DiskCacheSizeMb) {
            let bytes = megabytes(self.settings.uint(Setting::DiskCacheSizeMb));
            let api = Arc::clone(&self.api);
            tokio::spawn(async move { api.set_disk_cache_bytes(bytes).await });
        }
    }
}

impl EventListener for CacheSizes {
    fn on_event(&mut self, event: &AppEvent) {
        if let Some(changed) = event.settings() {
            self.apply(changed);
        }
    }
}
