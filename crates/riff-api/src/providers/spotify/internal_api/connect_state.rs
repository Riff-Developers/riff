use std::collections::HashMap;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::SpotifyInternalApi;
use crate::error::DomainError;
use crate::models::{Device, DeviceKind, RepeatMode};
use crate::remote::{
    QueueEntry, RemoteCluster, RemoteCommand, RemoteContext, RemoteEndpoint, RemotePlayback,
    RemotePush, TransferPlayback,
};

const CONNECTION_ID_HEADER: &str = "X-Spotify-Connection-Id";
const CLUSTER_URI: &str = "hm://connect-state/v1/cluster";
const CONNECTIONS_URI: &str = "hm://pusher/v1/connections/";
const TRACK_URI_PREFIX: &str = "spotify:track:";
const QUEUE_PROVIDER: &str = "queue";
const MAX_VOLUME: u32 = 65535;

pub(in crate::providers::spotify) async fn endpoint(
    api: &SpotifyInternalApi,
) -> Result<RemoteEndpoint, DomainError> {
    let token = api.access_token()?;
    let host = api.dealer_host().await;
    Ok(RemoteEndpoint {
        url: format!("wss://{host}/?access_token={token}"),
    })
}

pub(in crate::providers::spotify) async fn register_observer(
    api: &SpotifyInternalApi,
    observer_id: &str,
    connection_id: &str,
    name: &str,
) -> Result<RemoteCluster, DomainError> {
    let body = json!({
        "member_type": "CONNECT_STATE",
        "device": {"device_info": {
            "can_play": false,
            "volume": MAX_VOLUME,
            "name": name,
            "device_id": observer_id,
            "device_type": "COMPUTER",
            "capabilities": {
                "can_be_player": false,
                "hidden": true,
                "is_observable": true,
                "is_controllable": true,
                "supports_transfer_command": true,
                "supports_command_request": true,
                "command_acks": true,
                "volume_steps": 64,
                "needs_full_player_state": true,
                "supports_gzip_pushes": false,
            },
        }},
        "put_state_reason": "NEW_DEVICE",
        "is_active": false,
    });
    let reply = send_json(
        api,
        Method::PUT,
        &format!("/connect-state/v1/devices/{observer_id}"),
        body,
        connection_id,
    )
    .await?;
    let cluster: WireCluster =
        serde_json::from_slice(&reply).map_err(|e| DomainError::Parse(e.to_string()))?;
    Ok(cluster.into_model())
}

pub(in crate::providers::spotify) async fn unregister_observer(
    api: &SpotifyInternalApi,
    observer_id: &str,
    connection_id: &str,
) -> Result<(), DomainError> {
    api.send(
        Method::DELETE,
        &format!("/connect-state/v1/devices/{observer_id}"),
        "application/json",
        vec![],
        &[(CONNECTION_ID_HEADER, connection_id)],
    )
    .await
    .map(|_| ())
}

pub(in crate::providers::spotify) async fn command(
    api: &SpotifyInternalApi,
    from: &str,
    to: &str,
    connection_id: &str,
    command: RemoteCommand,
) -> Result<(), DomainError> {
    let path = format!("/connect-state/v1/player/command/from/{from}/to/{to}");
    for body in command_bodies(command) {
        let body = json!({ "command": body });
        send_json(api, Method::POST, &path, body, connection_id).await?;
    }
    Ok(())
}

pub(in crate::providers::spotify) async fn transfer(
    api: &SpotifyInternalApi,
    from: &str,
    to: &str,
    connection_id: &str,
    playback: TransferPlayback,
) -> Result<(), DomainError> {
    let path = format!("/connect-state/v1/connect/transfer/from/{from}/to/{to}");
    let restore_paused = match playback {
        TransferPlayback::Keep => "restore",
        TransferPlayback::Play => "resume",
        TransferPlayback::Pause => "pause",
    };
    let body = json!({ "transfer_options": { "restore_paused": restore_paused } });
    send_json(api, Method::POST, &path, body, connection_id)
        .await
        .map(|_| ())
}

pub(in crate::providers::spotify) async fn set_volume(
    api: &SpotifyInternalApi,
    from: &str,
    to: &str,
    connection_id: &str,
    percent: u8,
) -> Result<(), DomainError> {
    let path = format!("/connect-state/v1/connect/volume/from/{from}/to/{to}");
    let volume = percent.min(100) as u32 * MAX_VOLUME / 100;
    send_json(
        api,
        Method::PUT,
        &path,
        json!({ "volume": volume }),
        connection_id,
    )
    .await
    .map(|_| ())
}

async fn send_json(
    api: &SpotifyInternalApi,
    method: Method,
    path: &str,
    body: serde_json::Value,
    connection_id: &str,
) -> Result<Vec<u8>, DomainError> {
    api.send(
        method,
        path,
        "application/json",
        body.to_string().into_bytes(),
        &[
            (CONNECTION_ID_HEADER, connection_id),
            ("Accept", "application/json"),
        ],
    )
    .await
}

fn command_bodies(command: RemoteCommand) -> Vec<serde_json::Value> {
    let body = match command {
        RemoteCommand::Resume => json!({"endpoint": "resume"}),
        RemoteCommand::Pause => json!({"endpoint": "pause"}),
        RemoteCommand::SkipNext => json!({"endpoint": "skip_next"}),
        RemoteCommand::SkipPrevious => json!({"endpoint": "skip_prev"}),
        RemoteCommand::SeekTo(position_ms) => json!({"endpoint": "seek_to", "value": position_ms}),
        RemoteCommand::SetShuffle(shuffle) => {
            json!({"endpoint": "set_shuffling_context", "value": shuffle})
        }
        RemoteCommand::SetRepeat(mode) => {
            return vec![
                json!({"endpoint": "set_repeating_context", "value": mode != RepeatMode::Off}),
                json!({"endpoint": "set_repeating_track", "value": mode == RepeatMode::Track}),
            ];
        }
        RemoteCommand::PlayContext {
            context,
            track_id,
            track_index,
            position_ms,
            paused,
            shuffle,
        } => {
            let uri = context_uri(&context);
            let mut body = play(
                json!({"uri": uri, "url": format!("context://{uri}")}),
                track_index,
                position_ms,
                paused,
                shuffle,
            );
            if let Some(id) = track_id {
                body["options"]["skip_to"]["track_uri"] = json!(format!("{TRACK_URI_PREFIX}{id}"));
            }
            body
        }
        RemoteCommand::PlayTracks {
            track_ids,
            index,
            position_ms,
            paused,
            shuffle,
        } => {
            let tracks: Vec<_> = track_ids
                .iter()
                .map(|id| json!({"uri": format!("{TRACK_URI_PREFIX}{id}")}))
                .collect();
            play(
                json!({"uri": "", "pages": [{"tracks": tracks}]}),
                index,
                position_ms,
                paused,
                shuffle,
            )
        }
        RemoteCommand::AddToQueue { track_id } => json!({
            "endpoint": "add_to_queue",
            "track": {"uri": format!("{TRACK_URI_PREFIX}{track_id}")},
        }),
        RemoteCommand::SetQueue {
            next,
            previous,
            revision,
        } => json!({
            "endpoint": "set_queue",
            "next_tracks": next.into_iter().map(queue_edit_entry).collect::<Vec<_>>(),
            "prev_tracks": previous.into_iter().map(|e| e.wire).collect::<Vec<_>>(),
            "queue_revision": revision,
        }),
        RemoteCommand::SkipTo(entry) => json!({"endpoint": "skip_next", "track": entry.wire}),
    };
    vec![body]
}

fn play(
    context: serde_json::Value,
    index: Option<usize>,
    position_ms: u32,
    paused: bool,
    shuffle: Option<bool>,
) -> serde_json::Value {
    let mut options = json!({"seek_to": position_ms, "initially_paused": paused});
    if let Some(index) = index {
        options["skip_to"] = json!({"track_index": index});
    }
    if let Some(shuffle) = shuffle {
        options["player_options_override"] = json!({"shuffling_context": shuffle});
    }
    json!({
        "endpoint": "play",
        "context": context,
        "options": options,
        "play_origin": {"feature_identifier": "riff"},
    })
}

pub fn context_uri(context: &RemoteContext) -> String {
    match context {
        RemoteContext::Playlist(id) => format!("spotify:playlist:{id}"),
        RemoteContext::Album(id) => format!("spotify:album:{id}"),
        RemoteContext::Artist(id) => format!("spotify:artist:{id}"),
        RemoteContext::SavedTracks { user_id } => format!("spotify:user:{user_id}:collection"),
    }
}

fn parse_context_uri(uri: &str) -> Option<RemoteContext> {
    let mut parts = uri.strip_prefix("spotify:")?.split(':');
    match (parts.next()?, parts.next(), parts.next(), parts.next()) {
        ("playlist", Some(id), None, _) => Some(RemoteContext::Playlist(id.to_string())),
        ("album", Some(id), None, _) => Some(RemoteContext::Album(id.to_string())),
        ("artist", Some(id), None, _) => Some(RemoteContext::Artist(id.to_string())),
        ("user", Some(user_id), Some("collection"), None) => Some(RemoteContext::SavedTracks {
            user_id: user_id.to_string(),
        }),
        _ => None,
    }
}

pub(in crate::providers::spotify) fn parse_push(text: &str) -> RemotePush {
    let Ok(message) = serde_json::from_str::<WireMessage>(text) else {
        return RemotePush::Other;
    };
    let uri = message.uri.unwrap_or_default();
    if uri.starts_with(CONNECTIONS_URI) {
        if let Some(id) = message.headers.get("Spotify-Connection-Id") {
            return RemotePush::Connected {
                connection_id: id.clone(),
            };
        }
    }
    if uri == CLUSTER_URI {
        let update = message
            .payloads
            .into_iter()
            .find_map(|payload| serde_json::from_value::<WireClusterUpdate>(payload).ok());
        if let Some(update) = update {
            return RemotePush::Cluster(update.cluster.into_model());
        }
    }
    RemotePush::Other
}

pub(in crate::providers::spotify) const PING: &str = r#"{"type":"ping"}"#;

#[derive(Deserialize)]
struct WireMessage {
    uri: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    payloads: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct WireClusterUpdate {
    cluster: WireCluster,
}

#[derive(Default, Deserialize)]
struct WireCluster {
    #[serde(default, deserialize_with = "number")]
    timestamp: i64,
    active_device_id: Option<String>,
    player_state: Option<WirePlayerState>,
    #[serde(default)]
    devices: HashMap<String, WireDevice>,
}

#[derive(Default, Deserialize)]
struct WirePlayerState {
    #[serde(default, deserialize_with = "number")]
    timestamp: i64,
    context_uri: Option<String>,
    track: Option<WireTrack>,
    #[serde(default, deserialize_with = "number")]
    position_as_of_timestamp: i64,
    #[serde(default, deserialize_with = "number")]
    duration: i64,
    #[serde(default)]
    is_playing: bool,
    #[serde(default)]
    is_paused: bool,
    #[serde(default)]
    options: WireOptions,
    #[serde(default)]
    next_tracks: Vec<serde_json::Value>,
    #[serde(default)]
    prev_tracks: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "text")]
    queue_revision: String,
}

#[derive(Default, Deserialize)]
struct WireOptions {
    #[serde(default)]
    shuffling_context: bool,
    #[serde(default)]
    repeating_context: bool,
    #[serde(default)]
    repeating_track: bool,
}

#[derive(Default, Deserialize)]
struct WireTrack {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

#[derive(Default, Deserialize, Serialize)]
struct WireDevice {
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    device_type: String,
    #[serde(default)]
    volume: Option<u32>,
    #[serde(default)]
    capabilities: WireCapabilities,
}

#[derive(Default, Deserialize, Serialize)]
struct WireCapabilities {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    disable_volume: bool,
    #[serde(default)]
    restrict_to_local: bool,
    #[serde(default)]
    connect_disabled: bool,
    #[serde(default)]
    is_controllable: Option<bool>,
}

impl WireCapabilities {
    /// Whether other devices can control it, so it's offered as one to play
    /// on. A device that doesn't say whether it's controllable is taken to be.
    fn controllable(&self) -> bool {
        !self.hidden
            && !self.restrict_to_local
            && !self.connect_disabled
            && self.is_controllable != Some(false)
    }
}

/// An integer, which may arrive as a string: ones that can exceed 2^53 do.
fn number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Int(i64),
        Text(String),
    }
    Ok(match Number::deserialize(deserializer)? {
        Number::Int(n) => n,
        Number::Text(s) => s.parse().unwrap_or_default(),
    })
}

fn text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn queue_entry(wire: serde_json::Value) -> QueueEntry {
    let track = WireTrack::deserialize(&wire).unwrap_or_default();
    QueueEntry {
        track_id: track.track_id(),
        queued: track.queued(),
        wire,
    }
}

fn queue_edit_entry(entry: QueueEntry) -> serde_json::Value {
    let mut wire = entry.wire;
    if entry.queued {
        if let Some(fields) = wire.as_object_mut() {
            fields.insert("provider".to_string(), json!(QUEUE_PROVIDER));
            let metadata = fields.entry("metadata").or_insert_with(|| json!({}));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert("is_queued".to_string(), json!("true"));
                metadata.insert("provider".to_string(), json!(QUEUE_PROVIDER));
            }
        }
    }
    wire
}

impl WireTrack {
    fn queued(&self) -> bool {
        self.provider == QUEUE_PROVIDER
            || self.metadata.get("is_queued").is_some_and(|v| v == "true")
            || self
                .metadata
                .get("provider")
                .is_some_and(|v| v == QUEUE_PROVIDER)
    }

    fn track_id(&self) -> Option<String> {
        if self
            .metadata
            .get("hidden_in_queue")
            .is_some_and(|v| v == "true")
        {
            return None;
        }
        self.uri.strip_prefix(TRACK_URI_PREFIX).map(str::to_string)
    }
}

impl WireCluster {
    fn into_model(self) -> RemoteCluster {
        let active = self.active_device_id.filter(|id| !id.is_empty());
        let mut devices: Vec<Device> = self
            .devices
            .into_values()
            .filter(|d| d.capabilities.controllable())
            .map(WireDevice::into_model)
            .collect();
        devices.sort_by(|a, b| a.label.cmp(&b.label));
        let server_time = self.timestamp;
        let playback = self
            .player_state
            .filter(|_| active.is_some())
            .map(|state| state.into_model(server_time));
        RemoteCluster {
            active_device_id: active,
            devices,
            playback,
        }
    }
}

impl WirePlayerState {
    fn into_model(self, server_time: i64) -> RemotePlayback {
        let is_playing = self.is_playing && !self.is_paused;
        let elapsed = if is_playing && server_time > self.timestamp && self.timestamp > 0 {
            server_time - self.timestamp
        } else {
            0
        };
        let mut position = self.position_as_of_timestamp + elapsed;
        if self.duration > 0 {
            position = position.min(self.duration);
        }
        let repeat = if self.options.repeating_track {
            RepeatMode::Track
        } else if self.options.repeating_context {
            RepeatMode::Context
        } else {
            RepeatMode::Off
        };
        RemotePlayback {
            track_id: self.track.as_ref().and_then(WireTrack::track_id),
            context: self.context_uri.as_deref().and_then(parse_context_uri),
            is_playing,
            position_ms: position.max(0) as u32,
            duration_ms: (self.duration > 0).then_some(self.duration as u32),
            shuffle: self.options.shuffling_context,
            repeat,
            next: self.next_tracks.into_iter().map(queue_entry).collect(),
            previous: self.prev_tracks.into_iter().map(queue_entry).collect(),
            queue_revision: self.queue_revision,
        }
    }
}

impl WireDevice {
    fn into_model(self) -> Device {
        let kind = match self.device_type.as_str() {
            "SMARTPHONE" | "TABLET" => DeviceKind::Phone,
            "COMPUTER" => DeviceKind::Computer,
            "SPEAKER" | "AVR" | "STB" | "AUDIO_DONGLE" => DeviceKind::Speaker,
            "TV" | "CAST_VIDEO" | "GAME_CONSOLE" => DeviceKind::Tv,
            _ => DeviceKind::Other,
        };
        let supports_volume = !self.capabilities.disable_volume && self.volume.is_some();
        Device {
            id: self.device_id,
            label: self.name,
            kind,
            volume_percent: self
                .volume
                .map(|v| ((v.min(MAX_VOLUME) * 100 + MAX_VOLUME / 2) / MAX_VOLUME) as u8),
            supports_volume,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLUSTER_PUSH: &str = r#"{
        "type": "message",
        "uri": "hm://connect-state/v1/cluster",
        "headers": {"Content-Type": "application/json"},
        "payloads": [{
            "cluster": {
                "timestamp": "1760659205000",
                "active_device_id": "pixel",
                "player_state": {
                    "timestamp": "1760659200000",
                    "context_uri": "spotify:album:x",
                    "track": {"uri": "spotify:track:t2", "metadata": {}},
                    "position_as_of_timestamp": "30000",
                    "duration": "200000",
                    "is_playing": true,
                    "is_paused": false,
                    "options": {"shuffling_context": true, "repeating_context": true},
                    "queue_revision": "15130871412783076140",
                    "next_tracks": [
                        {"uri": "spotify:track:q1", "uid": "q0", "provider": "queue"},
                        {"uri": "spotify:track:q2", "uid": "q1", "provider": "context",
                            "metadata": {"iteration": "0", "provider": "queue"}},
                        {"uri": "spotify:track:t3", "metadata": {}},
                        {"uri": "spotify:track:gone", "metadata": {"hidden_in_queue": "true"}},
                        {"uri": "spotify:delimiter", "metadata": {}}
                    ],
                    "prev_tracks": [{"uri": "spotify:track:t1"}]
                },
                "devices": {
                    "pixel": {"device_id": "pixel", "name": "Pixel", "device_type": "SMARTPHONE",
                        "volume": 65535, "capabilities": {"disable_volume": true}},
                    "riff": {"device_id": "riff", "name": "Riff", "device_type": "COMPUTER",
                        "volume": 32768, "capabilities": {}},
                    "observer": {"device_id": "observer", "name": "Observer",
                        "device_type": "COMPUTER", "capabilities": {"hidden": true}},
                    "cast": {"device_id": "cast", "name": "Cast", "device_type": "CAST_VIDEO",
                        "capabilities": {"is_controllable": false}},
                    "local": {"device_id": "local", "name": "Local", "device_type": "SPEAKER",
                        "capabilities": {"restrict_to_local": true}},
                    "off": {"device_id": "off", "name": "Off", "device_type": "SPEAKER",
                        "capabilities": {"connect_disabled": true}}
                }
            },
            "update_reason": "DEVICE_STATE_CHANGED"
        }]
    }"#;

    #[test]
    fn reads_a_cluster_push() {
        let RemotePush::Cluster(cluster) = parse_push(CLUSTER_PUSH) else {
            panic!("expected a cluster");
        };
        assert_eq!(cluster.active_device_id.as_deref(), Some("pixel"));
        let names: Vec<_> = cluster.devices.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(names, ["Pixel", "Riff"]);
        let pixel = &cluster.devices[0];
        assert_eq!(pixel.kind, DeviceKind::Phone);
        assert!(!pixel.supports_volume);
        assert_eq!(cluster.devices[1].volume_percent, Some(50));

        let playback = cluster.playback.unwrap();
        assert_eq!(playback.track_id.as_deref(), Some("t2"));
        assert_eq!(
            playback.context,
            Some(RemoteContext::Album("x".to_string()))
        );
        assert!(playback.is_playing);
        assert_eq!(playback.position_ms, 35000);
        assert_eq!(playback.repeat, RepeatMode::Context);
        assert!(playback.shuffle);
        let next: Vec<_> = playback
            .next
            .iter()
            .filter_map(|e| e.track_id.as_deref())
            .collect();
        assert_eq!(next, ["q1", "q2", "t3"]);
        assert!(playback.next[0].queued);
        assert!(
            playback.next[1].queued,
            "queued, though only its metadata says so"
        );
        assert!(!playback.next[2].queued);
        assert_eq!(playback.next.len(), 5);
        assert_eq!(playback.previous[0].track_id.as_deref(), Some("t1"));
        assert_eq!(playback.queue_revision, "15130871412783076140");
    }

    #[test]
    fn queue_edits_send_entries_back_with_queued_ones_marked() {
        let RemotePush::Cluster(cluster) = parse_push(CLUSTER_PUSH) else {
            panic!("expected a cluster");
        };
        let playback = cluster.playback.unwrap();
        let mut next = playback.next.clone();
        next.swap(0, 2);
        let bodies = command_bodies(RemoteCommand::SetQueue {
            next,
            previous: playback.previous.clone(),
            revision: playback.queue_revision.clone(),
        });
        let tracks = &bodies[0]["next_tracks"];
        assert_eq!(bodies[0]["endpoint"], "set_queue");
        assert_eq!(tracks[0]["uri"], "spotify:track:t3");
        assert_eq!(
            tracks[0]["metadata"],
            json!({}),
            "a context entry is left as it came"
        );
        for queued in [&tracks[1], &tracks[2]] {
            assert_eq!(queued["provider"], "queue");
            assert_eq!(queued["metadata"]["is_queued"], "true");
            assert_eq!(queued["metadata"]["provider"], "queue");
        }
        assert_eq!(tracks[1]["uid"], "q1");
        assert_eq!(tracks[1]["metadata"]["iteration"], "0");
        assert_eq!(tracks[2]["uid"], "q0");
        assert_eq!(bodies[0]["prev_tracks"][0]["uri"], "spotify:track:t1");
        assert_eq!(bodies[0]["queue_revision"], "15130871412783076140");

        let skip = command_bodies(RemoteCommand::SkipTo(playback.next[2].clone()));
        assert_eq!(skip[0]["endpoint"], "skip_next");
        assert_eq!(skip[0]["track"]["uri"], "spotify:track:t3");
    }

    #[test]
    fn context_uris_round_trip() {
        for context in [
            RemoteContext::Playlist("p".to_string()),
            RemoteContext::Album("a".to_string()),
            RemoteContext::Artist("r".to_string()),
            RemoteContext::SavedTracks {
                user_id: "me".to_string(),
            },
        ] {
            assert_eq!(parse_context_uri(&context_uri(&context)), Some(context));
        }
        assert_eq!(parse_context_uri("spotify:web-api"), None);
        assert_eq!(parse_context_uri("spotify:user:me:playlist:x"), None);
        assert_eq!(parse_context_uri(""), None);
    }

    #[test]
    fn reads_the_connection_id() {
        let text = r#"{"type":"message","uri":"hm://pusher/v1/connections/abc",
            "headers":{"Spotify-Connection-Id":"abc"}}"#;
        assert_eq!(
            parse_push(text),
            RemotePush::Connected {
                connection_id: "abc".to_string()
            }
        );
        assert_eq!(parse_push(r#"{"type":"pong"}"#), RemotePush::Other);
        assert_eq!(parse_push("not json"), RemotePush::Other);
    }

    #[test]
    fn no_active_device_means_no_playback() {
        let cluster: WireCluster = serde_json::from_str(
            r#"{"active_device_id": "", "player_state": {"is_playing": true}}"#,
        )
        .unwrap();
        let cluster = cluster.into_model();
        assert_eq!(cluster.active_device_id, None);
        assert_eq!(cluster.playback, None);
    }

    #[test]
    fn play_options_are_left_out_unless_given() {
        let by_id = command_bodies(RemoteCommand::PlayContext {
            context: RemoteContext::Album("x".to_string()),
            track_id: Some("t2".to_string()),
            track_index: None,
            position_ms: 0,
            paused: false,
            shuffle: None,
        });
        assert_eq!(
            by_id[0]["options"]["skip_to"]["track_uri"],
            "spotify:track:t2"
        );
        assert!(by_id[0]["options"]["skip_to"].get("track_index").is_none());

        let from_start = command_bodies(RemoteCommand::PlayContext {
            context: RemoteContext::SavedTracks {
                user_id: "me".to_string(),
            },
            track_id: None,
            track_index: None,
            position_ms: 0,
            paused: false,
            shuffle: Some(true),
        });
        assert_eq!(
            from_start[0]["context"]["uri"],
            "spotify:user:me:collection"
        );
        let options = &from_start[0]["options"];
        assert!(options.get("skip_to").is_none());
        assert_eq!(
            options["player_options_override"]["shuffling_context"],
            true
        );

        let tracks = command_bodies(RemoteCommand::PlayTracks {
            track_ids: vec!["a".to_string(), "b".to_string()],
            index: Some(1),
            position_ms: 500,
            paused: true,
            shuffle: None,
        });
        assert_eq!(
            tracks[0]["context"]["pages"][0]["tracks"][1]["uri"],
            "spotify:track:b"
        );
        assert_eq!(tracks[0]["options"]["skip_to"]["track_index"], 1);
        assert_eq!(tracks[0]["options"]["initially_paused"], true);
        assert!(tracks[0]["options"]
            .get("player_options_override")
            .is_none());
    }

    #[test]
    fn repeat_track_sets_both_options() {
        let bodies = command_bodies(RemoteCommand::SetRepeat(RepeatMode::Track));
        assert_eq!(bodies[0]["endpoint"], "set_repeating_context");
        assert_eq!(bodies[0]["value"], true);
        assert_eq!(bodies[1]["endpoint"], "set_repeating_track");
        assert_eq!(bodies[1]["value"], true);
    }
}
