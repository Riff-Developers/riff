use std::sync::Arc;
use std::thread;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use riff_core::events::PlaybackChanged;
use riff_core::playback::{PlaybackAction, PlaybackState};

mod mpris;
mod server;
mod types;
mod updates;

use mpris::{RiffMpris, RiffMprisPlayer};
use updates::MprisUpdate;

#[derive(Clone, Debug)]
pub enum Command {
    Playback(PlaybackAction),
    Raise,
}

pub(crate) type Dispatch = Arc<dyn Fn(Command) -> bool + Send + Sync>;

pub fn start(dispatch: impl Fn(Command) -> bool + Send + Sync + 'static) -> Mpris {
    let dispatch: Dispatch = Arc::new(dispatch);
    let mpris = RiffMpris::new(Arc::clone(&dispatch));
    let player = RiffMprisPlayer::new(dispatch);

    let (sender, receiver) = unbounded();

    thread::spawn(move || {
        if let Err(e) = server::run(mpris, player, receiver) {
            log::warn!("Media controls are unavailable: {e}");
        }
    });

    Mpris { sender }
}

pub struct Mpris {
    sender: UnboundedSender<MprisUpdate>,
}

impl Mpris {
    pub fn on_playback_changed(&self, event: &PlaybackChanged, playback: &PlaybackState) {
        self.send(updates::update_for(event, playback));
    }

    pub fn on_queue_changed(&self, playback: &PlaybackState) {
        self.send(updates::queue_update(playback));
    }

    fn send(&self, update: MprisUpdate) {
        if self.sender.is_closed() {
            return;
        }
        if let Err(e) = self.sender.unbounded_send(update) {
            log::error!("Could not send event to DBUS server: {e}");
        }
    }
}
