use futures::channel::mpsc::UnboundedSender;
use riff_core::remote::Controller;

use crate::app::components::EventListener;
use crate::app::state::{
    Action, DeviceRequest, Event, LoginMethod, PlaybackAction, PlaybackChanged, PlaybackFields,
    SessionAction, SessionChanged, SessionFields, SessionStatus, Setting, SettingsChanged,
};
use crate::app::{AppAction, AppEvent, Dispatcher};
use riff_playback::player::Command;

use riff_config::SettingsStore;

pub struct PlayerNotifier {
    dispatcher: Dispatcher,
    command_sender: UnboundedSender<Command>,
    controller: Controller,
    settings: SettingsStore,
}

impl PlayerNotifier {
    pub fn new(
        dispatcher: Dispatcher,
        command_sender: UnboundedSender<Command>,
        controller: Controller,
    ) -> Self {
        Self {
            dispatcher,
            command_sender,
            controller,
            settings: SettingsStore::new(),
        }
    }

    fn apply_settings(&self, changed: &SettingsChanged) {
        let settings = &self.settings;
        if changed.contains(Setting::SkipExplicit) {
            let skip = settings.bool(Setting::SkipExplicit);
            self.dispatcher
                .dispatch(PlaybackAction::SetSkipExplicit(skip).into());
        }
        let effects = vec![
            changed
                .contains(Setting::MonoAudio)
                .then(|| Command::SetMono {
                    enabled: settings.bool(Setting::MonoAudio),
                }),
            changed.contains(Setting::Pan).then(|| Command::SetPan {
                pan: settings.double(Setting::Pan),
            }),
            changed
                .contains(Setting::PitchCents)
                .then(|| Command::SetPitch {
                    cents: settings.double(Setting::PitchCents),
                }),
            changed
                .contains_any(&Setting::EQ_BANDS)
                .then(|| Command::SetEqualizer {
                    bands: Setting::EQ_BANDS.map(|band| settings.double(band)),
                }),
        ];
        for command in effects.into_iter().flatten() {
            let _ = self.command_sender.unbounded_send(command);
        }
    }

    fn notify_login(&self, event: &SessionChanged) {
        if !event.changed.contains(SessionFields::STATUS) {
            return;
        }
        info!("notify_login: {:?}", event.status);
        let command = match event.status {
            SessionStatus::LoggingIn(LoginMethod::Saved) => Some(Command::Restore),
            SessionStatus::LoggingIn(LoginMethod::Browser) => Some(Command::InitLogin),
            SessionStatus::LoggedOut => Some(Command::Logout),
            _ => None,
        };
        if let Some(command) = command {
            self.send_command_to_local_player(command);
        }
        if event.status == SessionStatus::LoggedIn {
            self.controller.reconnect();
        }
    }

    fn sync_skip_explicit(&self, event: &PlaybackChanged) {
        if !event.changed.contains(PlaybackFields::SKIP_EXPLICIT) {
            return;
        }
        let skip = event.state.skip_explicit;
        self.settings.set_bool(Setting::SkipExplicit, skip).ok();
        self.controller.set_skip_explicit(skip);
        self.send_command_to_local_player(Command::SetSkipExplicit(skip));
    }

    fn send_command_to_local_player(&self, command: Command) {
        let dispatcher = &self.dispatcher;
        self.command_sender
            .unbounded_send(command)
            .unwrap_or_else(|_| {
                dispatcher.dispatch(AppAction::Core(Action::Session(SessionAction::LoginFailed)));
            });
    }
}

impl EventListener for PlayerNotifier {
    fn on_event(&mut self, event: &AppEvent) {
        let AppEvent::Core(event) = event else {
            return;
        };
        match event {
            Event::Session(event) => self.notify_login(event),
            Event::DeviceRequest(DeviceRequest::Control(intent)) => {
                self.controller.send(intent.clone())
            }
            Event::DeviceRequest(DeviceRequest::ThisDevice(id)) => {
                self.controller.set_this_device_id(id.clone())
            }
            Event::DeviceRequest(DeviceRequest::PlayerReport(report)) => {
                self.controller.report(report.clone())
            }
            Event::Playback(event) => self.sync_skip_explicit(event),
            Event::Settings(changed) => self.apply_settings(changed),
            Event::TokenRejected => self.send_command_to_local_player(Command::RefreshToken),
            Event::PlayerSettingsChanged => self.send_command_to_local_player(
                Command::ReloadSettings(crate::settings::player_settings().unwrap_or_default()),
            ),
            _ => {}
        }
    }
}
