use riff_api::models::Track;

use super::EntryKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueRole {
    Queued,
    Context,
    Current,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueSection {
    NowPlaying,
    Queued,
    Context(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueRow {
    pub key: EntryKey,
    pub track: Track,
    pub role: QueueRole,
    pub section: QueueSection,
}
