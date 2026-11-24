use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use reqwest::Client;
use riff_auth::PlayerKey;
use riff_core::events::EngineStatus;
use riff_core::schedule::{Every, Outcome, Scheduler};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;

use super::download::{self, InstallError};
use super::install_manager::{Build, InstallManager};
use super::protocol::{self, Event};
use super::recovery::Recovery;
use super::{unix_ms, Input, SoloistConfig, SoloistError};
use crate::player::PlayerEvents;
use riff_audio_engine::{Effects, EffectsNode};

const LET_GO_TIMEOUT: Duration = Duration::from_secs(2);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(20);
const PORT_POLL: Duration = Duration::from_millis(100);
const RETRIES: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];
const STABLE: Duration = Duration::from_secs(60);
const UPDATE_CHECK: Every = Every {
    period: Duration::from_secs(24 * 60 * 60),
    retry: Duration::from_secs(60 * 60),
};
const UPDATE_TASK: &str = "soloist-update";
const WARN_DAYS: i64 = 14;
const TRIAL_FAILURES: u32 = 2;
const EXIT_EXPIRED: i32 = 10;

pub(super) async fn run(
    config: SoloistConfig,
    mut inputs: UnboundedReceiver<Input>,
    device_id: Arc<Mutex<Option<String>>>,
    reported_signed_in: Arc<AtomicBool>,
    effects: Effects,
    events: PlayerEvents,
) {
    let (staged, staged_rx) = mpsc::unbounded_channel();
    let mut supervisor = Supervisor {
        installs: InstallManager::new(config.root.clone()),
        client: Client::new(),
        config,
        events,
        logged_in: false,
        key: None,
        key_refused: false,
        refused_key: None,
        device_id,
        effects,
        effects_node: None,
        playing: false,
        signed_in: None,
        prefetch: None,
        reported_signed_in,
        active: false,
        pending: None,
        trial_failures: None,
        staged,
        staged_rx,
        updates: None,
        warned_days: None,
        out_of_date: false,
        recovery: Recovery::default(),
        outbox: Vec::new(),
        stream_name: format!("riff-soloist-{:016x}", rand::random::<u64>()),
    };
    let mut failures = 0;
    loop {
        if !supervisor.logged_in {
            supervisor.updates = None;
            supervisor.set_status(EngineStatus::BuiltIn);
            if supervisor.wait_until(&mut inputs, |s| s.logged_in).await {
                return;
            }
            continue;
        }
        if supervisor.updates.is_none() {
            supervisor.updates = supervisor.schedule_updates();
        }
        if supervisor.key.is_none() {
            supervisor.key = api_key(supervisor.refused_key.as_deref()).await;
        }
        let Some(key) = supervisor.key.clone() else {
            warn!("There's no Soloist API key yet");
            let refused = std::mem::take(&mut supervisor.key_refused);
            supervisor.set_status(EngineStatus::NeedsKey {
                key_page: super::KEY_PAGE.to_string(),
                refused,
            });
            if supervisor
                .wait_until(&mut inputs, |s| s.key.is_some() || !s.logged_in)
                .await
            {
                return;
            }
            continue;
        };
        let binary = match supervisor.binary().await {
            Ok(binary) => Some(binary),
            Err(e) => {
                error!("Couldn't set up Soloist: {e}");
                None
            }
        };
        if let Some(binary) = binary {
            let started = Instant::now();
            let ended = if supervisor.out_of_date {
                info!("[dev] Soloist's build is out of date; not starting it");
                Ended::Exited(ExitStatus::from_raw(EXIT_EXPIRED << 8))
            } else {
                supervisor.run_once(&binary, &key, &mut inputs).await
            };
            let was_signed_in = supervisor.signed_in == Some(true);
            supervisor.set_signed_in(None);
            if supervisor.logged_in {
                let active = supervisor.active;
                supervisor
                    .recovery
                    .lost(active, std::time::Instant::now(), unix_ms());
            } else {
                supervisor.recovery.forget();
            }
            supervisor.active = false;
            match ended {
                Ended::AppGone => return,
                Ended::Restart => {
                    failures = 0;
                    continue;
                }
                Ended::Exited(status) if status.code() == Some(EXIT_EXPIRED) => {
                    if supervisor.replace_expired().await {
                        failures = 0;
                        continue;
                    }
                }
                Ended::Exited(status) => warn!("Soloist exited ({status})"),
                Ended::Failed(reason) => warn!("Soloist failed: {reason}"),
            }
            if was_signed_in {
                supervisor.events.set_connection_lost(true);
            }
            if started.elapsed() > STABLE {
                failures = 0;
            }
            supervisor.on_failure();
        }
        let Some(&delay) = RETRIES.get(failures) else {
            error!("Soloist couldn't be set up or started; giving up");
            supervisor.events.set_connection_lost(false);
            supervisor.set_status(EngineStatus::Failed);
            if supervisor.wait_until(&mut inputs, |s| !s.logged_in).await {
                return;
            }
            failures = 0;
            continue;
        };
        failures += 1;
        info!("Trying Soloist again in {delay:?}");
        if supervisor.wait(delay, &mut inputs).await {
            return;
        }
    }
}

enum Ended {
    AppGone,
    Restart,
    Exited(ExitStatus),
    Failed(String),
}

struct Supervisor {
    config: SoloistConfig,
    installs: InstallManager,
    client: Client,
    events: PlayerEvents,
    logged_in: bool,
    key: Option<String>,
    key_refused: bool,
    refused_key: Option<String>,
    device_id: Arc<Mutex<Option<String>>>,
    effects: Effects,
    effects_node: Option<EffectsNode>,
    playing: bool,
    signed_in: Option<bool>,
    prefetch: Option<JoinHandle<Result<Build, InstallError>>>,
    reported_signed_in: Arc<AtomicBool>,
    active: bool,
    pending: Option<Build>,
    trial_failures: Option<u32>,
    staged: UnboundedSender<Result<Option<Build>, InstallError>>,
    staged_rx: UnboundedReceiver<Result<Option<Build>, InstallError>>,
    updates: Option<Scheduler>,
    warned_days: Option<i64>,
    out_of_date: bool,
    recovery: Recovery,
    outbox: Vec<Value>,
    stream_name: String,
}

impl Supervisor {
    fn set_status(&self, status: EngineStatus) {
        self.events.set_engine(status);
    }

    fn set_signed_in(&mut self, signed_in: Option<bool>) {
        self.signed_in = signed_in;
        self.reported_signed_in
            .store(signed_in == Some(true), Ordering::Relaxed);
    }

    async fn take(&mut self, input: Input) -> bool {
        match input {
            Input::Command(command) => {
                warn!("Soloist isn't running; dropped {command}");
                false
            }
            Input::LoggedIn(logged_in) => {
                let stop = self.logged_in && !logged_in;
                self.logged_in = logged_in;
                if !logged_in {
                    self.key = None;
                }
                stop
            }
            Input::Prepare => {
                self.prefetch_build();
                false
            }
            Input::CheckForUpdate => {
                match &self.updates {
                    Some(updates) => {
                        info!("[dev] Looking for a newer Soloist build now");
                        updates.run_now(UPDATE_TASK);
                    }
                    None => {
                        warn!("[dev] Soloist isn't updated: logged out, or a build of your own")
                    }
                }
                false
            }
            Input::MakeOutOfDate => self.make_out_of_date(),
            Input::Kill => {
                warn!("[dev] Soloist isn't running; nothing to kill");
                false
            }
            Input::SetKey(key) => {
                let key = key.trim().to_string();
                if let Err(e) = PlayerKey::set(&key).await {
                    warn!("Couldn't save the Soloist API key: {e}");
                }
                let changed = self.key.as_deref() != Some(key.as_str());
                if self.refused_key.as_deref() != Some(key.as_str()) {
                    self.refused_key = None;
                }
                self.key = (!key.is_empty()).then_some(key);
                changed
            }
        }
    }

    async fn wait_until(
        &mut self,
        inputs: &mut UnboundedReceiver<Input>,
        ready: impl Fn(&Self) -> bool,
    ) -> bool {
        while !ready(self) {
            match inputs.recv().await {
                Some(input) => {
                    self.take(input).await;
                }
                None => return true,
            }
        }
        false
    }

    async fn wait(&mut self, delay: Duration, inputs: &mut UnboundedReceiver<Input>) -> bool {
        let until = Instant::now() + delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(until) => return false,
                input = inputs.recv() => match input {
                    Some(input) => {
                        if self.take(input).await {
                            return false;
                        }
                    }
                    None => return true,
                },
            }
        }
    }

    async fn start_effects(&mut self) {
        if self
            .effects_node
            .as_ref()
            .is_some_and(EffectsNode::is_alive)
        {
            return;
        }
        if self.effects_node.take().is_some() {
            warn!("The effects node went away; creating it again");
        }
        let (chain, source) = (self.effects.chain(), self.stream_name.clone());
        let started = tokio::task::spawn_blocking(move || EffectsNode::start(chain, source)).await;
        let error = match started {
            Ok(Ok(node)) => {
                self.effects_node = Some(node);
                return;
            }
            Ok(Err(e)) => e,
            Err(e) => e.to_string(),
        };
        warn!("Couldn't create the effects node ({error}); playing without effects");
    }

    async fn read_device_id(&self) {
        let file = self.config.state_dir().join(".device_id");
        let id = tokio::fs::read_to_string(&file)
            .await
            .ok()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        if id.is_none() {
            warn!("Soloist didn't say its device id; it can't be found among the devices");
        }
        *self.device_id.lock().unwrap() = id;
    }

    fn prefetch_build(&mut self) {
        if self.config.binary.is_some()
            || self.installs.current().is_some()
            || self
                .prefetch
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        info!("Downloading Soloist ahead, while signing in");
        let (installs, client) = (self.installs.clone(), self.client.clone());
        self.prefetch = Some(tokio::task::spawn(async move {
            download::ensure(&installs, &client).await
        }));
    }

    async fn binary(&mut self) -> Result<PathBuf, InstallError> {
        if let Some(binary) = &self.config.binary {
            return Ok(binary.clone());
        }
        if let Some(task) = self.prefetch.take() {
            if !task.is_finished() {
                self.set_status(EngineStatus::Installing);
            }
            match task.await {
                Ok(Ok(build)) => return Ok(self.installs.binary(&build)),
                Ok(Err(e)) => warn!("Downloading Soloist ahead failed: {e}"),
                Err(e) => warn!("Downloading Soloist ahead stopped: {e}"),
            }
        }
        if self.installs.current().is_none() {
            self.set_status(EngineStatus::Installing);
        }
        let build = download::ensure(&self.installs, &self.client).await?;
        Ok(self.installs.binary(&build))
    }

    fn make_out_of_date(&mut self) -> bool {
        if self.config.binary.is_some() {
            warn!("[dev] A build of your own isn't updated");
            return false;
        }
        let Some(current) = self.installs.current() else {
            warn!("[dev] Soloist isn't installed yet");
            return false;
        };
        let now = unix_ms() / 1000;
        let build = Build {
            etag: Some("\"dev-out-of-date\"".to_string()),
            last_modified: None,
            expires_at: Some(now.saturating_sub(24 * 60 * 60)),
            ..current
        };
        if let Err(e) = self.installs.note(&build) {
            warn!("[dev] Couldn't mark Soloist's build out of date: {e}");
            return false;
        }
        warn!("[dev] Soloist's build {} is now out of date", build.version);
        self.out_of_date = true;
        self.pending = None;
        true
    }

    async fn replace_expired(&mut self) -> bool {
        error!("Soloist's build expired; downloading a newer one");
        if self.config.binary.is_some() {
            return false;
        }
        self.set_status(EngineStatus::Updating);
        let current = self.installs.current().map(|build| build.id);
        match download::update(&self.installs, &self.client).await {
            Ok(Some(build)) if current.as_ref() == Some(&build.id) => {
                if !self.out_of_date {
                    warn!("The server's Soloist build is the expired one");
                    return false;
                }
                info!("[dev] Soloist {} is current again", build.version);
                self.out_of_date = false;
                self.pending = None;
                true
            }
            Ok(Some(build)) => {
                let switched = self.switch_to(&build);
                if switched {
                    self.out_of_date = false;
                    self.pending = None;
                }
                switched
            }
            Ok(None) => {
                warn!("There's no newer Soloist build yet");
                false
            }
            Err(e) => {
                warn!("Couldn't download a newer Soloist build: {e}");
                false
            }
        }
    }

    fn on_failure(&mut self) {
        let Some(failures) = self.trial_failures.as_mut() else {
            return;
        };
        *failures += 1;
        if *failures < TRIAL_FAILURES {
            return;
        }
        self.trial_failures = None;
        match self.installs.roll_back() {
            Ok(true) => warn!("The new Soloist build keeps stopping; went back to the last one"),
            Ok(false) => {}
            Err(e) => error!("Couldn't go back to the last Soloist build: {e}"),
        }
    }

    fn schedule_updates(&self) -> Option<Scheduler> {
        if self.config.binary.is_some() {
            return None;
        }
        let mut scheduler = Scheduler::new(self.config.run_log.clone());
        let (installs, client, staged) = (
            self.installs.clone(),
            self.client.clone(),
            self.staged.clone(),
        );
        scheduler.add(UPDATE_TASK, UPDATE_CHECK, move || {
            let (installs, client, staged) = (installs.clone(), client.clone(), staged.clone());
            async move {
                let Some(current) = installs.current() else {
                    return Outcome::Done;
                };
                if let Some(days) = download::days_left(&current) {
                    info!("Soloist's build has {days} days left");
                }
                let result = download::update(&installs, &client).await;
                let outcome = match result {
                    Ok(_) => Outcome::Done,
                    Err(_) => Outcome::Later,
                };
                let _ = staged.send(result);
                outcome
            }
        });
        Some(scheduler)
    }

    fn on_update(&mut self, result: Result<Option<Build>, InstallError>) {
        let current = self.installs.current().map(|build| build.id);
        match result {
            Ok(Some(build)) if current.as_ref() != Some(&build.id) => {
                info!(
                    "Soloist {} is ready; it's used once nothing plays",
                    build.version
                );
                self.warned_days = None;
                self.pending = Some(build);
            }
            Ok(_) => {
                debug!("Soloist is up to date");
                self.warned_days = None;
            }
            Err(e) => {
                warn!("Couldn't update Soloist: {e}");
                let days = self
                    .installs
                    .current()
                    .as_ref()
                    .and_then(download::days_left)
                    .filter(|&days| days <= WARN_DAYS);
                if let Some(days) = days.filter(|&days| self.warned_days != Some(days)) {
                    self.warned_days = Some(days);
                    self.events
                        .report_soloist_error(SoloistError::ExpiresSoon { days });
                }
            }
        }
    }

    fn switch_if_quiet(&mut self) -> bool {
        if self.playing || self.recovery.deadline().is_some() {
            return false;
        }
        let Some(build) = self.pending.take() else {
            return false;
        };
        self.switch_to(&build)
    }

    fn switch_to(&mut self, build: &Build) -> bool {
        match self.installs.switch_to(&build.id) {
            Ok(()) => {
                info!("Switching to Soloist {}", build.version);
                self.trial_failures = Some(0);
                true
            }
            Err(e) => {
                error!("Couldn't switch to the new Soloist build: {e}");
                false
            }
        }
    }

    async fn run_once(
        &mut self,
        binary: &Path,
        key: &str,
        inputs: &mut UnboundedReceiver<Input>,
    ) -> Ended {
        let state_dir = self.config.state_dir();
        for dir in [&state_dir, &self.config.cache_dir] {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                return Ended::Failed(format!("couldn't create {}: {e}", dir.display()));
            }
        }
        stop_stale(&state_dir).await;
        let port_file = state_dir.join("ws.port");
        let _ = tokio::fs::remove_file(&port_file).await;

        self.start_effects().await;
        let effects = self
            .effects_node
            .as_ref()
            .filter(|node| node.is_alive())
            .map(|_| self.stream_name.as_str());
        let mut child = match spawn(binary, key, &self.config, effects) {
            Ok(child) => child,
            Err(e) => return Ended::Failed(format!("couldn't start {}: {e}", binary.display())),
        };
        info!("Soloist started as {}", self.config.device_name);
        let (hints, mut hints_rx) = mpsc::unbounded_channel();
        forward_logs(&mut child, hints);

        let port = tokio::select! {
            port = read_port(&port_file) => port,
            status = child.wait() => return exited(status),
        };
        let Some(port) = port else {
            return Ended::Failed("its socket didn't open".to_string());
        };
        self.read_device_id().await;
        let effects = self.effects_node.as_ref().map(EffectsNode::alive);
        let url = format!("ws://127.0.0.1:{port}");
        let mut socket = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((socket, _)) => socket,
            Err(e) => return Ended::Failed(format!("couldn't connect to {url}: {e}")),
        };
        debug!("Connected to Soloist on {url}");

        loop {
            tokio::select! {
                status = child.wait() => return exited(status),
                message = socket.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let stop = self.on_event(&text);
                        if let Err(e) = self.flush(&mut socket).await {
                            return Ended::Failed(e);
                        }
                        if stop {
                            let _ = child.kill().await;
                            return Ended::Restart;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Ended::Failed("its socket closed".to_string())
                    }
                    Some(Err(e)) => return Ended::Failed(format!("its socket failed: {e}")),
                    Some(Ok(_)) => {}
                },
                input = inputs.recv() => match input {
                    Some(Input::Kill) => {
                        warn!("[dev] Killing Soloist");
                        if let Err(e) = child.start_kill() {
                            warn!("[dev] Couldn't kill Soloist: {e}");
                        }
                    }
                    Some(Input::Command(command)) => {
                        debug!("Soloist: {command}");
                        if let Err(e) = socket.send(Message::Text(command.to_string().into())).await {
                            return Ended::Failed(format!("couldn't send a command: {e}"));
                        }
                    }
                    Some(input) => {
                        if self.take(input).await {
                            info!("Stopping Soloist: the account or its key changed");
                            if !self.logged_in && self.active {
                                self.let_go(&mut socket).await;
                            }
                            let _ = child.kill().await;
                            return Ended::Restart;
                        }
                    }
                    None => {
                        let _ = child.kill().await;
                        return Ended::AppGone;
                    }
                },
                _ = gone(effects.clone()) => {
                    warn!("The effects node went away; starting Soloist again with a new one");
                    let _ = child.kill().await;
                    return Ended::Restart;
                }
                _ = until(self.recovery.deadline()) => {
                    let again = self.recovery.timed_out(std::time::Instant::now());
                    self.outbox.extend(again);
                    if let Err(e) = self.flush(&mut socket).await {
                        return Ended::Failed(e);
                    }
                }
                Some(LogHint::KeyRefused) = hints_rx.recv() => {
                    warn!("The service refused the Soloist API key; asking for it again");
                    if let Err(e) = PlayerKey::clear().await {
                        warn!("Couldn't remove the refused key: {e}");
                    }
                    self.refused_key = self.key.take();
                    self.key_refused = true;
                    let _ = child.kill().await;
                    return Ended::Restart;
                }
                Some(result) = self.staged_rx.recv() => {
                    self.on_update(result);
                    if self.switch_if_quiet() {
                        let _ = child.kill().await;
                        return Ended::Restart;
                    }
                }
            }
        }
    }

    async fn let_go(&mut self, socket: &mut Socket) {
        let deactivate = protocol::deactivate().to_string();
        if let Err(e) = socket.send(Message::Text(deactivate.into())).await {
            warn!("Couldn't ask Soloist to give up playback: {e}");
            return;
        }
        let gone = async {
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Text(text) = message {
                    if let Some(Event::DeviceChanged {
                        is_active: false, ..
                    }) = protocol::parse(&text)
                    {
                        return;
                    }
                }
            }
        };
        match timeout(LET_GO_TIMEOUT, gone).await {
            Ok(()) => {
                info!("Soloist gave up playback");
                self.active = false;
            }
            Err(_) => warn!("Soloist didn't give up playback in time"),
        }
    }

    fn settle(&mut self, now_ms: u64) {
        let moves = self
            .recovery
            .settle(self.active, std::time::Instant::now(), now_ms);
        self.outbox.extend(moves);
    }

    async fn flush(&mut self, socket: &mut Socket) -> Result<(), String> {
        for message in std::mem::take(&mut self.outbox) {
            debug!("Soloist: {message}");
            socket
                .send(Message::Text(message.to_string().into()))
                .await
                .map_err(|e| format!("couldn't send a command: {e}"))?;
        }
        Ok(())
    }

    fn on_event(&mut self, text: &str) -> bool {
        let Some(event) = protocol::parse(text) else {
            return false;
        };
        let now_ms = unix_ms();
        self.recovery.follow(&event, now_ms);
        if let Some(playback) = event.playback() {
            self.playing = playback.status.playing();
            if let Some(active) = playback.is_active {
                self.active = active;
            }
            self.settle(now_ms);
            return self.switch_if_quiet();
        }
        match event {
            Event::AuthState {
                logged_in,
                is_active,
                device_name,
            } => {
                info!("Soloist as {device_name}: signed in: {logged_in}, active: {is_active}");
                if logged_in {
                    self.trial_failures = None;
                }
                self.events.set_connection_lost(false);
                let settings = riff_config::SettingsStore::new();
                let paired = settings.soloist_paired();
                if logged_in {
                    if !paired {
                        settings.set_soloist_paired(true);
                    }
                    self.set_status(EngineStatus::Ready);
                } else if paired && self.signed_in.is_none() {
                    debug!("Soloist is restoring its pairing");
                } else {
                    if paired {
                        settings.set_soloist_paired(false);
                    }
                    self.set_status(EngineStatus::NeedsPairing { device_name });
                }
                self.set_signed_in(Some(logged_in));
                self.active = logged_in && is_active;
                if logged_in {
                    let take_back = self.recovery.signed_in(std::time::Instant::now());
                    self.outbox.extend(take_back);
                }
            }
            Event::DeviceChanged {
                is_active,
                device_name,
            } => {
                info!("Soloist as {device_name}: active: {is_active}");
                self.active = is_active;
                self.settle(now_ms);
            }
            Event::CommandResult { command } => debug!("Soloist took {command}"),
            Event::Error { message } => warn!("Soloist refused a command: {message}"),
            _ => {}
        }
        false
    }
}

async fn gone(alive: Option<std::sync::Arc<AtomicBool>>) {
    let Some(alive) = alive else {
        return std::future::pending().await;
    };
    while alive.load(Ordering::Relaxed) {
        sleep(Duration::from_secs(1)).await;
    }
}

async fn until(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
}

async fn api_key(refused: Option<&str>) -> Option<String> {
    let usable = |key: &String| !key.is_empty() && Some(key.as_str()) != refused;
    let from_env = std::env::var("RIFF_SOLOIST_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(usable);
    let saved = PlayerKey::get().await.filter(usable);
    match from_env {
        Some(key) => {
            if saved.as_deref() != Some(key.as_str()) {
                if let Err(e) = PlayerKey::set(&key).await {
                    warn!("Couldn't save the Soloist API key: {e}");
                }
            }
            Some(key)
        }
        None => saved,
    }
}

fn spawn(
    binary: &Path,
    key: &str,
    config: &SoloistConfig,
    effects: Option<&str>,
) -> std::io::Result<Child> {
    let mut command = Command::new(binary);
    command
        .arg("--device-name")
        .arg(&config.device_name)
        .arg("--api-key")
        .arg(key)
        .arg("--data-dir")
        .arg(config.state_dir())
        .arg("--cache-dir")
        .arg(&config.cache_dir)
        .arg("--cache-size")
        .arg(super::support::cache_size_mb().to_string())
        .arg("--ws")
        .arg("127.0.0.1:0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(name) = effects {
        command
            .env("PIPEWIRE_AUTOCONNECT", "false")
            .env("PIPEWIRE_PROPS", format!("{{ node.name = {name} }}"));
    }
    if cfg!(debug_assertions) {
        command.arg("--verbose");
    }
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    command.spawn()
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

enum LogHint {
    KeyRefused,
}

const KEY_REFUSED_LINE: &str = "login failed while API key is configured";

fn forward_logs(child: &mut Child, hints: UnboundedSender<LogHint>) {
    fn forward(stream: impl AsyncRead + Unpin + Send + 'static, hints: UnboundedSender<LogHint>) {
        tokio::task::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!("soloist: {line}");
                if line.contains(KEY_REFUSED_LINE) {
                    let _ = hints.send(LogHint::KeyRefused);
                }
            }
        });
    }
    if let Some(stdout) = child.stdout.take() {
        forward(stdout, hints.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        forward(stderr, hints);
    }
}

async fn stop_stale(state_dir: &Path) {
    let Ok(text) = tokio::fs::read_to_string(state_dir.join("soloist.pid")).await else {
        return;
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return;
    };
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    if comm.trim() != "soloist" {
        return;
    }
    warn!("Stopping a Soloist left running ({pid})");
    unsafe { libc::kill(pid, libc::SIGTERM) };
    for _ in 0..20 {
        sleep(Duration::from_millis(100)).await;
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
    }
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

async fn read_port(file: &Path) -> Option<u16> {
    timeout(SOCKET_TIMEOUT, async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(file).await {
                if let Ok(port) = text.trim().parse() {
                    return port;
                }
            }
            sleep(PORT_POLL).await;
        }
    })
    .await
    .ok()
}

fn exited(status: std::io::Result<ExitStatus>) -> Ended {
    match status {
        Ok(status) => Ended::Exited(status),
        Err(e) => Ended::Failed(format!("couldn't wait for it: {e}")),
    }
}
