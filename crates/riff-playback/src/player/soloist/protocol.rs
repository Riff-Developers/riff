use riff_api::models::RepeatMode;
use riff_api::remote::RemoteCommand;
use riff_api::ContentUris;
use serde::Deserialize;
use serde_json::{json, Value};

pub fn plays(command: &RemoteCommand, uris: &ContentUris) -> bool {
    match command {
        RemoteCommand::Resume
        | RemoteCommand::Pause
        | RemoteCommand::SkipNext
        | RemoteCommand::SkipPrevious
        | RemoteCommand::SeekTo(_)
        | RemoteCommand::SetShuffle(_)
        | RemoteCommand::SetRepeat(_) => true,
        RemoteCommand::AddToQueue { track_id } => uris.track(track_id).is_some(),
        RemoteCommand::PlayContext {
            context,
            track_id,
            track_index,
            position_ms,
            paused,
            shuffle,
            ..
        } => {
            track_id.is_none()
                && track_index.is_none()
                && *position_ms == 0
                && !paused
                && shuffle.is_none()
                && uris.context(context).is_some()
        }
        RemoteCommand::PlayTracks { .. }
        | RemoteCommand::SetQueue { .. }
        | RemoteCommand::SkipTo(_) => false,
    }
}

pub fn commands(command: RemoteCommand, uris: &ContentUris) -> Vec<Value> {
    if !plays(&command, uris) {
        warn!("Soloist has no command for {command:?}");
        return vec![];
    }
    match command {
        RemoteCommand::Resume => vec![play()],
        RemoteCommand::Pause => vec![pause()],
        RemoteCommand::SkipNext => vec![message("skip_next", json!({}))],
        RemoteCommand::SkipPrevious => vec![message("skip_prev", json!({}))],
        RemoteCommand::SeekTo(position_ms) => vec![seek(position_ms as u64)],
        RemoteCommand::SetShuffle(enabled) => {
            vec![message("set_shuffle", json!({ "enabled": enabled }))]
        }
        RemoteCommand::SetRepeat(mode) => {
            let repeat = |name, enabled| message(name, json!({ "enabled": enabled }));
            match mode {
                RepeatMode::Off => vec![
                    repeat("set_repeat_track", false),
                    repeat("set_repeat_context", false),
                ],
                RepeatMode::Context => vec![
                    repeat("set_repeat_track", false),
                    repeat("set_repeat_context", true),
                ],
                RepeatMode::Track => vec![
                    repeat("set_repeat_context", false),
                    repeat("set_repeat_track", true),
                ],
            }
        }
        RemoteCommand::AddToQueue { track_id } => uris
            .track(&track_id)
            .map(|uri| message("add_to_queue", json!({ "uri": uri })))
            .into_iter()
            .collect(),
        RemoteCommand::PlayContext { context, .. } => uris
            .context(&context)
            .map(|uri| message("play", json!({ "uri": uri })))
            .into_iter()
            .collect(),
        RemoteCommand::PlayTracks { .. }
        | RemoteCommand::SetQueue { .. }
        | RemoteCommand::SkipTo(_) => vec![],
    }
}

pub fn play() -> Value {
    message("play", json!({}))
}

pub fn pause() -> Value {
    message("pause", json!({}))
}

pub fn seek(position_ms: u64) -> Value {
    message("seek", json!({ "position_ms": position_ms }))
}

pub fn set_volume(percent: u8) -> Value {
    message("set_volume", json!({ "volume": percent.min(100) }))
}

pub fn activate() -> Value {
    message("activate", json!({}))
}

pub fn deactivate() -> Value {
    message("deactivate", json!({}))
}

fn message(command: &str, fields: Value) -> Value {
    let mut message = json!({ "type": "command", "command": command });
    if let (Some(message), Value::Object(fields)) = (message.as_object_mut(), fields) {
        message.extend(fields);
    }
    message
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    AuthState {
        logged_in: bool,
        #[serde(default)]
        is_active: bool,
        #[serde(default)]
        device_name: String,
    },
    DeviceChanged {
        is_active: bool,
        #[serde(default)]
        device_name: String,
    },
    CommandResult {
        command: String,
    },
    Error {
        message: String,
    },
    PlaybackState(Playback),
    PlaybackChanged(Playback),
    PositionSync {
        position: Position,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Playback {
    pub status: Status,
    #[serde(default)]
    pub item: Option<Item>,
    #[serde(default)]
    pub position: Option<Position>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Item {
    pub uri: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct Position {
    pub position_ms: u64,
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Playing,
    Paused,
    Buffering,
    #[serde(other)]
    Unknown,
}

impl Status {
    pub fn playing(self) -> bool {
        matches!(self, Self::Playing | Self::Buffering)
    }
}

impl Event {
    pub fn playback(&self) -> Option<&Playback> {
        match self {
            Self::PlaybackState(playback) | Self::PlaybackChanged(playback) => Some(playback),
            _ => None,
        }
    }
}

pub fn parse(text: &str) -> Option<Event> {
    serde_json::from_str(text)
        .map_err(|e| debug!("Soloist sent something unexpected ({e}): {text}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use riff_api::remote::RemoteContext;

    fn uris() -> ContentUris {
        ContentUris::new(
            |context| match context {
                RemoteContext::Album(id) => Some(format!("spotify:album:{id}")),
                _ => None,
            },
            |id| Some(format!("spotify:track:{id}")),
        )
    }

    fn play_context(track_id: Option<&str>) -> RemoteCommand {
        RemoteCommand::PlayContext {
            context: RemoteContext::Album("4aawyAB9vmqN3uQ7FjRGTy".to_string()),
            track_id: track_id.map(str::to_string),
            track_index: None,
            position_ms: 0,
            paused: false,
            shuffle: None,
        }
    }

    #[test]
    fn commands_are_tagged_messages() {
        assert_eq!(
            commands(RemoteCommand::SeekTo(30_000), &uris()),
            [json!({ "type": "command", "command": "seek", "position_ms": 30000 })]
        );
        assert_eq!(
            commands(RemoteCommand::Pause, &uris()),
            [json!({ "type": "command", "command": "pause" })]
        );
        assert_eq!(
            activate(),
            json!({ "type": "command", "command": "activate" })
        );
        assert_eq!(
            deactivate(),
            json!({ "type": "command", "command": "deactivate" })
        );
        assert_eq!(
            set_volume(150),
            json!({ "type": "command", "command": "set_volume", "volume": 100 })
        );
    }

    #[test]
    fn repeat_turns_the_other_mode_off_first() {
        let names = |mode| -> Vec<(String, bool)> {
            commands(RemoteCommand::SetRepeat(mode), &uris())
                .iter()
                .map(|m| {
                    (
                        m["command"].as_str().unwrap().to_string(),
                        m["enabled"].as_bool().unwrap(),
                    )
                })
                .collect()
        };
        assert_eq!(
            names(RepeatMode::Track),
            [
                ("set_repeat_context".to_string(), false),
                ("set_repeat_track".to_string(), true)
            ]
        );
        assert_eq!(
            names(RepeatMode::Off),
            [
                ("set_repeat_track".to_string(), false),
                ("set_repeat_context".to_string(), false)
            ]
        );
    }

    #[test]
    fn only_whole_contexts_play_over_the_socket() {
        assert!(plays(&play_context(None), &uris()));
        assert_eq!(
            commands(play_context(None), &uris()),
            [
                json!({ "type": "command", "command": "play", "uri": "spotify:album:4aawyAB9vmqN3uQ7FjRGTy" })
            ]
        );
        let from_a_track = play_context(Some("2JRo0gjbX4GrCqBYdRohoo"));
        assert!(!plays(&from_a_track, &uris()));
        assert!(commands(from_a_track, &uris()).is_empty());
        assert!(!plays(
            &RemoteCommand::PlayTracks {
                track_ids: vec!["a".to_string()],
                index: None,
                position_ms: 0,
                paused: false,
                shuffle: None,
            },
            &uris()
        ));
        assert!(plays(
            &RemoteCommand::AddToQueue {
                track_id: "a".to_string()
            },
            &uris()
        ));
    }

    #[test]
    fn reads_the_events_it_acts_on() {
        let auth = r#"{"type":"auth_state","logged_in":true,"is_active":false,"device_name":"Riff on desk"}"#;
        assert_eq!(
            parse(auth),
            Some(Event::AuthState {
                logged_in: true,
                is_active: false,
                device_name: "Riff on desk".to_string()
            })
        );
        assert_eq!(
            parse(r#"{"type":"device_changed","is_active":true,"device_name":"Riff"}"#),
            Some(Event::DeviceChanged {
                is_active: true,
                device_name: "Riff".to_string()
            })
        );
        assert_eq!(
            parse(r#"{"type":"error","message":"command requires authentication"}"#),
            Some(Event::Error {
                message: "command requires authentication".to_string()
            })
        );
        assert_eq!(
            parse(r#"{"type":"position_sync","position":{"position_ms":1}}"#),
            Some(Event::PositionSync {
                position: Position {
                    position_ms: 1,
                    timestamp_ms: None,
                    speed: None
                }
            })
        );
        assert_eq!(
            parse(r#"{"type":"queue_changed","tracks":[]}"#),
            Some(Event::Other)
        );
        assert_eq!(parse("not json"), None);
        let changed = parse(r#"{"type":"playback_changed","status":"paused"}"#).unwrap();
        assert!(!changed.playback().unwrap().status.playing());
        let state = parse(r#"{"type":"playback_state","status":"buffering","volume":65}"#);
        assert!(state.unwrap().playback().unwrap().status.playing());
    }

    #[test]
    fn reads_where_playback_is() {
        let state = r#"{"type":"playback_state","status":"paused",
            "item":{"uri":"spotify:track:3KvxPIAPSH7SHiovnx4znN","entity_type":"track",
                "playback":{"duration_ms":343000}},
            "context":{"uri":"spotify:album:5muITeHcGHTMh4mDUpCEwI"},
            "position":{"position_ms":1764,"timestamp_ms":1794891412039,"speed":0},
            "volume":40,"is_active":false}"#;
        assert_eq!(
            parse(state),
            Some(Event::PlaybackState(Playback {
                status: Status::Paused,
                item: Some(Item {
                    uri: "spotify:track:3KvxPIAPSH7SHiovnx4znN".to_string()
                }),
                position: Some(Position {
                    position_ms: 1764,
                    timestamp_ms: Some(1794891412039),
                    speed: Some(0.0)
                }),
                is_active: Some(false),
            }))
        );
    }
}
