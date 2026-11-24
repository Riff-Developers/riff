use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use riff_api::models::{ContentRating, Track};
use riff_api::remote::RemoteCommand;
use riff_api::{ApiService, DomainError, Load, TrackLookup};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::position::Position;

use super::local::{LocalSession, PlayerReport, LOCAL_DEVICE_ID};
use super::session::{RemoteSession, Request, SessionEvent};
use super::view::{
    plays_all_but_queue_edits, route, view, CurrentDevice, Intent, PlaybackView, Playing, Route,
    State,
};

const MAX_NEXT: usize = 50;
const LOOKUP_RETRY: Duration = Duration::from_secs(30);

pub trait LocalDevice: Send + Sync + 'static {
    fn command(&self, command: RemoteCommand);
    fn set_volume(&self, percent: u8);

    fn plays(&self, command: &RemoteCommand) -> bool {
        plays_all_but_queue_edits(command)
    }

    fn listed_as(&self) -> Option<String> {
        None
    }

    fn take_playback(&self) -> bool {
        false
    }

    fn skips_explicit(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub enum Update {
    View(Box<PlaybackView>),
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Offline,
    Unreachable,
    Refused,
    ThisDeviceUnlisted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Remote,
    Local,
}

impl SessionKind {
    pub fn for_provider(api: &ApiService) -> Self {
        if api.capabilities().remote_devices {
            Self::Remote
        } else {
            Self::Local
        }
    }
}

enum Input {
    Intent(Intent),
    ThisDevice(Option<String>),
    Reconnect,
    Report(PlayerReport),
    SkipExplicit(bool),
    Resolved {
        asked: Vec<String>,
        found: TrackLookup,
    },
}

enum Session {
    Remote(RemoteSession),
    Local(LocalSession),
}

impl Session {
    fn start(
        kind: SessionKind,
        api: &Arc<ApiService>,
        local: &Arc<dyn LocalDevice>,
        load: fn() -> Load,
    ) -> (Self, UnboundedReceiver<SessionEvent>) {
        match kind {
            SessionKind::Remote => {
                let (session, events) = RemoteSession::start(Arc::clone(api));
                (Self::Remote(session), events)
            }
            SessionKind::Local => {
                let (session, events) =
                    LocalSession::start(Arc::clone(api), Arc::clone(local), load);
                (Self::Local(session), events)
            }
        }
    }

    fn request(&self, request: Request) {
        match self {
            Self::Remote(session) => session.request(request),
            Self::Local(session) => session.request(request),
        }
    }
}

pub struct Controller {
    inputs: UnboundedSender<Input>,
}

impl Controller {
    pub fn start(
        api: Arc<ApiService>,
        local: Arc<dyn LocalDevice>,
        kind: SessionKind,
        load: fn() -> Load,
    ) -> (Self, UnboundedReceiver<Update>) {
        let (inputs, input_rx) = mpsc::unbounded_channel();
        let (updates, update_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(
            api,
            local,
            kind,
            load,
            inputs.clone(),
            input_rx,
            updates,
        ));
        (Self { inputs }, update_rx)
    }

    pub fn send(&self, intent: Intent) {
        let _ = self.inputs.send(Input::Intent(intent));
    }

    pub fn set_this_device_id(&self, id: Option<String>) {
        let _ = self.inputs.send(Input::ThisDevice(id));
    }

    pub fn reconnect(&self) {
        let _ = self.inputs.send(Input::Reconnect);
    }

    pub fn report(&self, report: PlayerReport) {
        let _ = self.inputs.send(Input::Report(report));
    }

    pub fn set_skip_explicit(&self, skip: bool) {
        let _ = self.inputs.send(Input::SkipExplicit(skip));
    }
}

struct Run {
    api: Arc<ApiService>,
    local: Arc<dyn LocalDevice>,
    load: fn() -> Load,
    session: Session,
    inputs: UnboundedSender<Input>,
    updates: UnboundedSender<Update>,
    state: State,
    online: bool,
    shown: Option<PlaybackView>,
    tracks: HashMap<String, Track>,
    looking_up: HashSet<String>,
    lookup_failed: HashMap<String, Instant>,
    player_playing: bool,
    played_here: Option<(Playing, Instant)>,
    skip_explicit: bool,
    skipped: Option<String>,
}

async fn run(
    api: Arc<ApiService>,
    local: Arc<dyn LocalDevice>,
    kind: SessionKind,
    load: fn() -> Load,
    input_tx: UnboundedSender<Input>,
    mut inputs: UnboundedReceiver<Input>,
    updates: UnboundedSender<Update>,
) {
    let (session, mut events) = Session::start(kind, &api, &local, load);
    let mut state = State::default();
    if kind == SessionKind::Local {
        state.this_device_id = Some(LOCAL_DEVICE_ID.to_string());
    }
    let mut run = Run {
        api,
        local,
        load,
        session,
        inputs: input_tx,
        updates,
        state,
        online: false,
        shown: None,
        tracks: HashMap::new(),
        looking_up: HashSet::new(),
        lookup_failed: HashMap::new(),
        player_playing: false,
        played_here: None,
        skip_explicit: false,
        skipped: None,
    };
    run.show();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(event) => run.on_session(event),
                None => return,
            },
            input = inputs.recv() => match input {
                Some(input) => run.on_input(input),
                None => return,
            },
        }
    }
}

impl Run {
    fn on_session(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Online(online) => self.online = online,
            SessionEvent::Cluster(cluster) => {
                if cluster.active_device_id.is_some() {
                    self.state.selected = None;
                }
                self.state.cluster = cluster;
                self.find_this_device();
            }
            SessionEvent::RequestFailed(error) => self.fail(classify(&error)),
        }
        self.show();
    }

    fn on_input(&mut self, input: Input) {
        match input {
            Input::Intent(intent) => self.apply(intent),
            Input::ThisDevice(id) => {
                if let Session::Remote(_) = self.session {
                    if self.local.listed_as().is_none() {
                        self.state.this_device_id = id;
                    }
                }
            }
            Input::Reconnect => {
                if let Session::Remote(session) = &self.session {
                    session.reconnect();
                }
            }
            Input::Report(report) => {
                self.player_playing = matches!(report, PlayerReport::Playing { .. });
                if let Session::Local(session) = &self.session {
                    session.report(report);
                }
            }
            Input::SkipExplicit(skip) => self.skip_explicit = skip,
            Input::Resolved { asked, found } => {
                let TrackLookup { found, error } = found;
                if let Some(e) = &error {
                    warn!("remote playback: couldn't look up tracks: {e}");
                }
                let now = Instant::now();
                for id in asked {
                    self.looking_up.remove(&id);
                    if found.contains_key(&id) || error.is_none() {
                        self.lookup_failed.remove(&id);
                    } else {
                        self.lookup_failed.insert(id, now);
                    }
                }
                self.tracks.extend(found);
            }
        }
        self.show();
    }

    fn apply(&mut self, intent: Intent) {
        let Some(mut route) = route(intent, &self.state, |command| self.local.plays(command))
        else {
            return;
        };
        debug!("remote playback: {route:?}");
        if let Route::TakePlayback { otherwise } = route {
            if self.local.take_playback() {
                return;
            }
            route = *otherwise;
        }
        let local = matches!(self.session, Session::Local(_));
        let request = match route {
            Route::Local(command) if local => Request::Command {
                to: LOCAL_DEVICE_ID.to_string(),
                command,
            },
            Route::LocalVolume(percent) if local => Request::Volume {
                to: LOCAL_DEVICE_ID.to_string(),
                percent,
            },
            Route::Local(command) => return self.local.command(command),
            Route::LocalVolume(percent) => return self.local.set_volume(percent),
            Route::Select(to) => {
                self.state.selected = to;
                return;
            }
            Route::ThisDeviceUnlisted => return self.fail(Failure::ThisDeviceUnlisted),
            Route::TakePlayback { .. } => return,
            Route::Send(request) => request,
        };
        if self.online {
            self.session.request(request);
        } else {
            self.fail(Failure::Offline);
        }
    }

    fn find_this_device(&mut self) {
        let Some(own_id) = self.local.listed_as() else {
            return;
        };
        let id = self
            .state
            .cluster
            .devices
            .iter()
            .any(|device| device.id == own_id)
            .then_some(own_id);
        if id != self.state.this_device_id {
            info!("this device is listed as {id:?}");
            self.state.this_device_id = id;
        }
    }

    fn fail(&self, failure: Failure) {
        let _ = self.updates.send(Update::Failed(failure));
    }

    fn show(&mut self) {
        let mut shown = view(&self.state, self.online);
        self.hold_playback_here(&mut shown);
        if let Some(playing) = shown.playback.as_mut() {
            playing.next.truncate(MAX_NEXT);
            let wanted: Vec<String> = playing
                .track_id
                .iter()
                .chain(playing.next.iter().map(|n| &n.track_id))
                .cloned()
                .collect();
            self.resolve(&wanted);
            playing.track = playing
                .track_id
                .as_ref()
                .and_then(|id| self.tracks.get(id))
                .cloned();
            for next in &mut playing.next {
                next.track = self.tracks.get(&next.track_id).cloned();
            }
        }
        if shown.current == CurrentDevice::ThisDevice {
            self.played_here = shown
                .playback
                .clone()
                .map(|playing| (playing, Instant::now()));
        }
        self.skip_explicit_here(&shown);
        if self.shown.as_ref() != Some(&shown) {
            self.shown = Some(shown.clone());
            let _ = self.updates.send(Update::View(Box::new(shown)));
        }
    }

    fn hold_playback_here(&self, shown: &mut PlaybackView) {
        if shown.playback.is_some()
            || shown.current != CurrentDevice::ThisDevice
            || !self.player_playing
        {
            return;
        }
        let Some((playing, at)) = &self.played_here else {
            return;
        };
        let mut playing = playing.clone();
        if playing.is_playing {
            let position = Position::running_since(playing.position_ms, *at).now();
            playing.position_ms = playing.duration_ms.map_or(position, |d| position.min(d));
        }
        shown.playback = Some(playing);
    }

    fn resolve(&mut self, wanted: &[String]) {
        self.tracks.retain(|id, _| wanted.contains(id));
        let now = Instant::now();
        self.lookup_failed
            .retain(|_, failed| now.duration_since(*failed) < LOOKUP_RETRY);
        let missing: Vec<String> = wanted
            .iter()
            .filter(|id| {
                !self.tracks.contains_key(*id)
                    && !self.looking_up.contains(*id)
                    && !self.lookup_failed.contains_key(*id)
            })
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        self.looking_up.extend(missing.iter().cloned());
        let (api, load, inputs) = (Arc::clone(&self.api), self.load, self.inputs.clone());
        tokio::spawn(async move {
            let found = api.get_tracks(&missing, load()).await;
            let _ = inputs.send(Input::Resolved {
                asked: missing,
                found,
            });
        });
    }

    fn skip_explicit_here(&mut self, shown: &PlaybackView) {
        let playing = shown.playback.as_ref();
        let current = playing.and_then(|p| p.track_id.as_ref());
        if self.skipped.as_ref() != current {
            self.skipped = None;
        }
        if !self.skip_explicit
            || shown.current != CurrentDevice::ThisDevice
            || self.local.skips_explicit()
            || self.skipped.is_some()
        {
            return;
        }
        let explicit = playing
            .and_then(|p| p.track.as_ref())
            .is_some_and(|track| track.content_rating == ContentRating::Explicit);
        if explicit {
            info!("remote playback: skipping an explicit track");
            self.skipped = current.cloned();
            self.apply(Intent::Next);
        }
    }
}

fn classify(error: &DomainError) -> Failure {
    match error {
        DomainError::Network(_) | DomainError::NoToken | DomainError::AuthExpired => {
            Failure::Offline
        }
        DomainError::ClientError { status: 403, .. } => Failure::Refused,
        _ => Failure::Unreachable,
    }
}
