use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gettextrs::{gettext, ngettext};
use riff_playback::player::soloist::SoloistError;
use riff_playback::player::SpotifyError;
use riff_playback::{PlayerError, PlayerEvent};

use crate::app::AppAction;

pub fn forward_player_events(actions: UnboundedSender<AppAction>) -> UnboundedSender<PlayerEvent> {
    let (sender, receiver) = unbounded::<PlayerEvent>();
    std::thread::spawn(move || {
        futures::executor::block_on(receiver.for_each(|event| {
            let _ = actions.unbounded_send(to_action(event));
            async {}
        }))
    });
    sender
}

fn to_action(event: PlayerEvent) -> AppAction {
    match event {
        PlayerEvent::Playback(action) => action.into(),
        PlayerEvent::Session(action) => action.into(),
        PlayerEvent::Error(PlayerError::Player(SpotifyError::PlaybackDrmBlocked)) => {
            AppAction::ShowDrmBlockedDialog
        }
        PlayerEvent::Error(PlayerError::Player(SpotifyError::PlaybackTemporarilyUnavailable)) => {
            AppAction::ShowNotification(gettext("Playback is Temporarily Unavailable"))
        }
        PlayerEvent::Error(PlayerError::Player(error)) => {
            AppAction::ShowNotification(format!("{error}"))
        }
        PlayerEvent::Error(PlayerError::Soloist(error)) => {
            AppAction::ShowNotification(soloist_error(&error))
        }
    }
}

pub fn soloist_error(error: &SoloistError) -> String {
    match error {
        SoloistError::ExpiresSoon { days } if *days <= 0 => {
            gettext("Soloist stops working today, and an update couldn’t be downloaded")
        }
        SoloistError::ExpiresSoon { days } => ngettext(
            "Soloist stops working in {} day, and an update couldn’t be downloaded",
            "Soloist stops working in {} days, and an update couldn’t be downloaded",
            (*days).clamp(0, i64::from(u32::MAX)) as u32,
        )
        .replace("{}", &days.to_string()),
    }
}
