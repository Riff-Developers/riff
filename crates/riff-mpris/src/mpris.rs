#![allow(non_snake_case)]
#![allow(unused_variables)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::TryInto;

use zbus::fdo::{Error, Result};
use zbus::interface;
use zbus::object_server::{Interface, SignalEmitter};
use zbus::zvariant::{ObjectPath, Value};

use riff_api::models::RepeatMode;
use riff_core::playback::PlaybackAction;

use crate::types::*;
use crate::updates::{is_jump, MprisUpdate};
use crate::{Command, Dispatch};

fn send(dispatch: &Dispatch, command: Command) -> Result<()> {
    if dispatch(command) {
        Ok(())
    } else {
        Err(Error::Failed("Could not send action".to_string()))
    }
}

pub struct RiffMpris {
    dispatch: Dispatch,
}

impl RiffMpris {
    pub fn new(dispatch: Dispatch) -> Self {
        Self { dispatch }
    }
}

#[interface(interface = "org.mpris.MediaPlayer2")]
impl RiffMpris {
    fn quit(&self) -> Result<()> {
        Err(Error::NotSupported("Not implemented".to_string()))
    }

    fn raise(&self) -> Result<()> {
        send(&self.dispatch, Command::Raise)
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &'static str {
        "Riff"
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &'static str {
        "dev.diegovsky.Riff"
    }
}

pub struct RiffMprisPlayer {
    state: MprisState,
    dispatch: Dispatch,
}

impl RiffMprisPlayer {
    pub fn new(dispatch: Dispatch) -> Self {
        Self {
            state: MprisState::new(),
            dispatch,
        }
    }

    pub async fn apply(
        &mut self,
        update: MprisUpdate,
        ctxt: &SignalEmitter<'_>,
    ) -> zbus::Result<()> {
        let can_go_next = self.can_go_next();
        let can_go_previous = self.can_go_previous();
        let can_seek = self.can_seek();
        let status = self.playback_status();
        let mut changed: HashMap<&str, Value> = HashMap::new();

        if let Some(track) = update.track {
            self.state.set_current_track(track);
            changed.insert("Metadata", Value::from(self.metadata()));
        }
        if let Some(new_status) = update.status {
            self.state.set_playing(new_status);
            if new_status != status {
                changed.insert("PlaybackStatus", Value::from(new_status));
            }
        }
        if let Some(loop_status) = update.loop_status {
            self.state.set_loop_status(loop_status);
            changed.insert("LoopStatus", Value::from(loop_status));
        }
        if let Some(shuffled) = update.shuffled {
            self.state.set_shuffled(shuffled);
            changed.insert("Shuffle", Value::from(shuffled));
        }
        if let Some(volume) = update.volume {
            self.state.set_volume(volume);
            changed.insert("Volume", Value::from(volume));
        }
        let mut seeked = None;
        if let Some(position) = update.position {
            let expected = self.state.position();
            self.state.set_position(position.micros);
            if is_jump(expected, position.micros, position.by_user) {
                seeked = Some(position.micros as i64);
            }
        }
        self.state.set_has_prev(update.has_prev);
        self.state.set_has_next(update.has_next);
        for (name, before, now) in [
            ("CanGoNext", can_go_next, self.can_go_next()),
            ("CanGoPrevious", can_go_previous, self.can_go_previous()),
            ("CanSeek", can_seek, self.can_seek()),
        ] {
            if before != now {
                changed.insert(name, Value::from(now));
            }
        }

        if !changed.is_empty() {
            zbus::fdo::Properties::properties_changed(
                ctxt,
                Self::name(),
                changed,
                Cow::Borrowed(&[]),
            )
            .await?;
        }
        if let Some(position) = seeked {
            Self::seeked(ctxt, position).await?;
        }
        Ok(())
    }
}

#[interface(interface = "org.mpris.MediaPlayer2.Player")]
impl RiffMprisPlayer {
    pub fn next(&self) -> Result<()> {
        send(&self.dispatch, Command::Playback(PlaybackAction::Next))
    }

    pub fn open_uri(&self, Uri: &str) -> Result<()> {
        Err(Error::NotSupported("Not implemented".to_string()))
    }

    pub fn pause(&self) -> Result<()> {
        send(&self.dispatch, Command::Playback(PlaybackAction::Pause))
    }

    pub fn play(&self) -> Result<()> {
        send(&self.dispatch, Command::Playback(PlaybackAction::Play))
    }

    pub fn play_pause(&self) -> Result<()> {
        send(
            &self.dispatch,
            Command::Playback(PlaybackAction::TogglePlay),
        )
    }

    pub fn previous(&self) -> Result<()> {
        send(&self.dispatch, Command::Playback(PlaybackAction::Previous))
    }

    pub fn seek(&self, Offset: i64) -> Result<()> {
        if self.state.current_track().is_none() {
            return Ok(());
        }

        let mut new_pos: i128 = (self.state.position() as i128 + i128::from(Offset)) / 1000;
        // As per spec, if new position is less than 0 round to 0
        if new_pos < 0 {
            new_pos = 0;
        }

        let new_pos: u32 = (new_pos)
            .try_into()
            .map_err(|_| Error::Failed("Could not parse position".to_string()))?;

        // As per spec, if new position is past the length of the song skip to
        // the next song
        if u64::from(new_pos) >= self.metadata().length / 1000 {
            send(&self.dispatch, Command::Playback(PlaybackAction::Next))
        } else {
            send(
                &self.dispatch,
                Command::Playback(PlaybackAction::Seek(new_pos)),
            )
        }
    }

    pub fn set_position(&self, TrackId: ObjectPath, Position: i64) -> Result<()> {
        if self.state.current_track().is_none() {
            return Ok(());
        }

        if Position < 0 || TrackId.as_str() != self.metadata().id {
            return Ok(());
        }

        let length: i64 = self
            .metadata()
            .length
            .try_into()
            .map_err(|_| Error::Failed("Could not cast length (too large)".to_string()))?;

        if Position > length {
            return Ok(());
        }

        let pos: u32 = (Position / 1000)
            .try_into()
            .map_err(|_| Error::Failed("Could not parse position".to_string()))?;

        send(&self.dispatch, Command::Playback(PlaybackAction::Seek(pos)))
    }

    pub fn stop(&self) -> Result<()> {
        Err(Error::NotSupported("Not implemented".to_string()))
    }

    #[zbus(signal)]
    pub async fn seeked(ctxt: &SignalEmitter<'_>, Position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    pub fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    pub fn can_go_next(&self) -> bool {
        self.state.has_next()
    }

    #[zbus(property)]
    pub fn can_go_previous(&self) -> bool {
        self.state.has_prev()
    }

    #[zbus(property)]
    pub fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    pub fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    pub fn can_seek(&self) -> bool {
        self.state.current_track().is_some()
    }

    #[zbus(property)]
    pub fn maximum_rate(&self) -> f64 {
        1.0f64
    }

    #[zbus(property)]
    pub fn metadata(&self) -> TrackMetadata {
        self.state
            .current_track()
            .cloned()
            .unwrap_or_else(|| TrackMetadata {
                id: NO_TRACK.to_owned(),
                length: 0,
                title: "Not playing".to_string(),
                artist: vec![],
                album: String::new(),
                art: None,
            })
    }

    #[zbus(property)]
    pub fn minimum_rate(&self) -> f64 {
        1.0f64
    }

    #[zbus(property)]
    pub fn playback_status(&self) -> PlaybackStatus {
        self.state.status()
    }

    #[zbus(property)]
    pub fn loop_status(&self) -> LoopStatus {
        self.state.loop_status()
    }

    #[zbus(property)]
    pub fn set_loop_status(&self, value: LoopStatus) -> zbus::Result<()> {
        let mode = match value {
            LoopStatus::None => RepeatMode::Off,
            LoopStatus::Track => RepeatMode::Track,
            LoopStatus::Playlist => RepeatMode::Context,
        };
        send(
            &self.dispatch,
            Command::Playback(PlaybackAction::SetRepeatMode(mode)),
        )?;
        Ok(())
    }

    #[zbus(property)]
    pub fn position(&self) -> i64 {
        self.state.position() as i64
    }

    #[zbus(property)]
    pub fn rate(&self) -> f64 {
        1.0f64
    }

    #[zbus(property)]
    pub fn set_rate(&self, value: f64) {}

    #[zbus(property)]
    pub fn shuffle(&self) -> bool {
        self.state.is_shuffled()
    }

    #[zbus(property)]
    pub fn set_shuffle(&self, value: bool) -> zbus::Result<()> {
        send(
            &self.dispatch,
            Command::Playback(PlaybackAction::SetShuffle(value)),
        )?;
        Ok(())
    }

    #[zbus(property)]
    pub fn volume(&self) -> f64 {
        self.state.volume()
    }

    #[zbus(property)]
    pub fn set_volume(&self, value: f64) -> zbus::Result<()> {
        // As per spec, if new volume less than 0 round to 0
        // also, we don't support volume higher than 100% at the moment.
        let volume = value.clamp(0.0, 1.0);
        send(
            &self.dispatch,
            Command::Playback(PlaybackAction::SetVolume(volume)),
        )?;
        Ok(())
    }
}
