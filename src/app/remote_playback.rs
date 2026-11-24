use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use riff_api::ApiService;
use riff_core::load;
use riff_core::remote::{Controller, LocalDevice, SessionKind, Update};
use riff_core::schedule::RunLog;
use riff_playback::player::soloist::{Soloist, SoloistConfig};
use riff_playback::PlayerEvent;

use crate::app::state::PlaybackAction;
use crate::app::AppAction;

pub fn start_remote_playback(
    api: Arc<ApiService>,
    this_device: Arc<dyn LocalDevice>,
    actions: UnboundedSender<AppAction>,
) -> Controller {
    let (started, controller) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("remote-playback")
            .enable_all()
            .build()
            .expect("couldn't start the playback runtime");
        runtime.block_on(async move {
            let kind = session_kind(&api);
            info!("playback is kept {kind:?}");
            let (controller, mut updates) =
                Controller::start(api, this_device, kind, load::transport);
            let _ = started.send(controller);
            while let Some(update) = updates.recv().await {
                let action = match update {
                    Update::View(view) => PlaybackAction::Show(view),
                    Update::Failed(failure) => PlaybackAction::Failed(failure),
                };
                if actions.unbounded_send(action.into()).is_err() {
                    break;
                }
            }
        });
    });
    controller
        .recv()
        .expect("the playback controller didn't start")
}

fn session_kind(api: &ApiService) -> SessionKind {
    #[cfg(debug_assertions)]
    if std::env::var_os("RIFF_LOCAL_PLAYBACK").is_some_and(|v| v == "1") {
        return SessionKind::Local;
    }
    SessionKind::for_provider(api)
}

pub fn start_soloist(
    api: &ApiService,
    events: UnboundedSender<PlayerEvent>,
    effects: riff_audio_engine::Effects,
    run_log: RunLog,
) -> Soloist {
    let config = SoloistConfig::new(
        glib::user_data_dir().join("riff").join("soloist"),
        glib::user_cache_dir().join("riff").join("soloist"),
        run_log,
    );
    Soloist::start(config, api.uris(), events, effects)
}
