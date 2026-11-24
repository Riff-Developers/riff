#[macro_use]
extern crate log;

pub mod player;

use riff_core::playback::PlaybackAction;
use riff_core::session::SessionAction;

#[derive(Debug)]
pub enum PlayerEvent {
    Playback(PlaybackAction),
    Session(SessionAction),
    Error(PlayerError),
}

#[derive(Debug)]
pub enum PlayerError {
    Player(player::SpotifyError),
    Soloist(player::soloist::SoloistError),
}

impl From<PlaybackAction> for PlayerEvent {
    fn from(action: PlaybackAction) -> Self {
        Self::Playback(action)
    }
}

impl From<SessionAction> for PlayerEvent {
    fn from(action: SessionAction) -> Self {
        Self::Session(action)
    }
}
