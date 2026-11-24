use bitflags::bitflags;
use riff_api::models::RepeatMode;

use crate::remote::{Intent, PlayerReport};

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct PlaybackFields: u16 {
        const STATUS = 1 << 0;
        const TRACK = 1 << 1;
        const CONTEXT = 1 << 2;
        const POSITION = 1 << 3;
        const SHUFFLE = 1 << 4;
        const REPEAT = 1 << 5;
        const VOLUME = 1 << 6;
        const SKIP_EXPLICIT = 1 << 7;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionOrigin {
    User,
    Player,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub track: Option<String>,
    pub position: u32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub volume: f64,
    pub skip_explicit: bool,
    pub local: bool,
}

impl PlaybackSnapshot {
    pub fn is_playing(&self) -> bool {
        self.status == PlaybackStatus::Playing
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackChanged {
    pub changed: PlaybackFields,
    pub state: PlaybackSnapshot,
    pub position_origin: Option<PositionOrigin>,
}

impl PlaybackChanged {
    pub fn seeked_by_user(&self) -> bool {
        self.changed.contains(PlaybackFields::POSITION)
            && self.position_origin == Some(PositionOrigin::User)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueChanged;

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct DeviceFields: u8 {
        const AVAILABLE = 1 << 0;
        const CURRENT = 1 << 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevicesChanged {
    pub changed: DeviceFields,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceRequest {
    Control(Intent),
    ThisDevice(Option<String>),
    PlayerReport(PlayerReport),
}
