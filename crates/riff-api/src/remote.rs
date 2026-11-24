use crate::models::{Device, RepeatMode};

#[derive(Clone, Copy, Debug)]
pub struct ContentUris {
    context: fn(&RemoteContext) -> Option<String>,
    track: fn(&str) -> Option<String>,
}

impl ContentUris {
    pub const fn new(
        context: fn(&RemoteContext) -> Option<String>,
        track: fn(&str) -> Option<String>,
    ) -> Self {
        Self { context, track }
    }

    /// For a provider whose players aren't told what to play by name.
    pub const fn none() -> Self {
        Self::new(|_| None, |_| None)
    }

    pub fn context(&self, context: &RemoteContext) -> Option<String> {
        (self.context)(context)
    }

    pub fn track(&self, track_id: &str) -> Option<String> {
        (self.track)(track_id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub remote_devices: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEndpoint {
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemotePush {
    Connected { connection_id: String },
    Cluster(RemoteCluster),
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteCluster {
    pub active_device_id: Option<String>,
    pub devices: Vec<Device>,
    pub playback: Option<RemotePlayback>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RemoteContext {
    Playlist(String),
    Album(String),
    Artist(String),
    SavedTracks { user_id: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemotePlayback {
    pub track_id: Option<String>,
    pub context: Option<RemoteContext>,
    pub is_playing: bool,
    pub position_ms: u32,
    pub duration_ms: Option<u32>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub next: Vec<QueueEntry>,
    pub previous: Vec<QueueEntry>,
    pub queue_revision: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueEntry {
    pub track_id: Option<String>,
    pub queued: bool,
    pub(crate) wire: serde_json::Value,
}

impl QueueEntry {
    pub fn keyed(track_id: &str, queued: bool, key: String) -> Self {
        Self {
            track_id: Some(track_id.to_string()),
            queued,
            wire: serde_json::Value::String(key),
        }
    }

    pub fn key(&self) -> Option<&str> {
        self.wire.as_str()
    }

    #[doc(hidden)]
    pub fn for_test(track_id: Option<&str>, queued: bool) -> Self {
        Self {
            track_id: track_id.map(str::to_string),
            queued,
            wire: serde_json::Value::String(track_id.unwrap_or("hidden").to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteCommand {
    Resume,
    Pause,
    SkipNext,
    SkipPrevious,
    SeekTo(u32),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    PlayContext {
        context: RemoteContext,
        track_id: Option<String>,
        track_index: Option<usize>,
        position_ms: u32,
        paused: bool,
        shuffle: Option<bool>,
    },
    PlayTracks {
        track_ids: Vec<String>,
        index: Option<usize>,
        position_ms: u32,
        paused: bool,
        shuffle: Option<bool>,
    },
    AddToQueue {
        track_id: String,
    },
    SetQueue {
        next: Vec<QueueEntry>,
        previous: Vec<QueueEntry>,
        revision: String,
    },
    SkipTo(QueueEntry),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferPlayback {
    Keep,
    Play,
    Pause,
}
