use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use riff_api::remote::{RemoteCluster, RemoteCommand, RemotePush, TransferPlayback};
use riff_api::{ApiService, DomainError};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::{interval, timeout, Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;

const KEEPALIVE: Duration = Duration::from_secs(30);
const SILENCE_LIMIT: Duration = Duration::from_secs(90);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(60);
const FINISH_SENDING: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Command {
        to: String,
        command: RemoteCommand,
    },
    Transfer {
        to: String,
        playback: TransferPlayback,
    },
    Volume {
        to: String,
        percent: u8,
    },
}

#[derive(Debug)]
pub enum SessionEvent {
    Online(bool),
    Cluster(RemoteCluster),
    RequestFailed(DomainError),
}

enum Control {
    Request(Request),
    Reconnect,
    Shutdown,
}

pub struct RemoteSession {
    controls: UnboundedSender<Control>,
}

impl RemoteSession {
    pub fn start(api: Arc<ApiService>) -> (Self, UnboundedReceiver<SessionEvent>) {
        let (controls, control_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(api, control_rx, events));
        (Self { controls }, event_rx)
    }

    pub fn request(&self, request: Request) {
        let _ = self.controls.send(Control::Request(request));
    }

    pub fn reconnect(&self) {
        let _ = self.controls.send(Control::Reconnect);
    }
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        let _ = self.controls.send(Control::Shutdown);
    }
}

enum Ended {
    Shutdown,
    Reconnect,
    Lost(String),
}

async fn run(
    api: Arc<ApiService>,
    mut controls: UnboundedReceiver<Control>,
    events: UnboundedSender<SessionEvent>,
) {
    let observer_id = uuid::Uuid::new_v4().simple().to_string();
    let mut retry = FIRST_RETRY;
    loop {
        let connected = Instant::now();
        let ended = connect(&api, &observer_id, &mut controls, &events).await;
        let _ = events.send(SessionEvent::Online(false));
        match ended {
            Ended::Shutdown => return,
            Ended::Reconnect => retry = FIRST_RETRY,
            Ended::Lost(reason) => {
                if connected.elapsed() > MAX_RETRY {
                    retry = FIRST_RETRY;
                }
                warn!("remote playback connection lost ({reason}); retrying in {retry:?}");
                if wait(retry, &mut controls, &events).await {
                    return;
                }
                retry = (retry * 2).min(MAX_RETRY);
            }
        }
    }
}

async fn wait(
    delay: Duration,
    controls: &mut UnboundedReceiver<Control>,
    events: &UnboundedSender<SessionEvent>,
) -> bool {
    let until = Instant::now() + delay;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(until) => return false,
            control = controls.recv() => match control {
                None | Some(Control::Shutdown) => return true,
                Some(Control::Reconnect) => return false,
                Some(Control::Request(_)) => offline(events),
            },
        }
    }
}

fn offline(events: &UnboundedSender<SessionEvent>) {
    let error = DomainError::Network("not connected".to_string());
    let _ = events.send(SessionEvent::RequestFailed(error));
}

async fn connect(
    api: &Arc<ApiService>,
    observer_id: &str,
    controls: &mut UnboundedReceiver<Control>,
    events: &UnboundedSender<SessionEvent>,
) -> Ended {
    let endpoint = match api.remote_endpoint().await {
        Ok(endpoint) => endpoint,
        Err(e) => return Ended::Lost(format!("no endpoint: {e}")),
    };
    let (mut socket, _) = match tokio_tungstenite::connect_async(endpoint.url.as_str()).await {
        Ok(connection) => connection,
        Err(e) => return Ended::Lost(format!("connecting: {e}")),
    };

    let connection_id = match timeout(READY_TIMEOUT, ready(api, &mut socket)).await {
        Ok(Some(id)) => id,
        Ok(None) => return Ended::Lost("closed before it was ready".to_string()),
        Err(_) => return Ended::Lost("not ready in time".to_string()),
    };
    match api.remote_register(observer_id, &connection_id).await {
        Ok(cluster) => {
            info!("remote playback connected");
            let _ = events.send(SessionEvent::Online(true));
            let _ = events.send(SessionEvent::Cluster(cluster));
        }
        Err(e) => return Ended::Lost(format!("registering: {e}")),
    }

    let ended = serve(
        api,
        observer_id,
        &connection_id,
        &mut socket,
        controls,
        events,
    )
    .await;
    let _ = socket.close(None).await;
    if let Err(e) = api.remote_unregister(observer_id, &connection_id).await {
        debug!("remote playback: couldn't unregister: {e}");
    }
    ended
}

async fn ready<S>(api: &ApiService, socket: &mut S) -> Option<String>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(Ok(message)) = socket.next().await {
        if let Message::Text(text) = message {
            if let RemotePush::Connected { connection_id } = api.parse_remote_push(&text) {
                return Some(connection_id);
            }
        }
    }
    None
}

async fn serve<S>(
    api: &Arc<ApiService>,
    observer_id: &str,
    connection_id: &str,
    socket: &mut S,
    controls: &mut UnboundedReceiver<Control>,
    events: &UnboundedSender<SessionEvent>,
) -> Ended
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message>
        + Unpin,
{
    let (requests, mut pending) = mpsc::unbounded_channel::<Request>();
    let mut sending = false;
    let ended = {
        let work = async {
            while let Some(request) = pending.recv().await {
                sending = true;
                if let Err(error) = send(api, observer_id, connection_id, request.clone()).await {
                    warn!("remote playback request {request:?} failed: {error}");
                    let _ = events.send(SessionEvent::RequestFailed(error));
                }
                sending = false;
            }
        };
        tokio::pin!(work);
        let ended = tokio::select! {
            ended = serve_pushes(api, socket, controls, events, &requests) => ended,
            () = &mut work => Ended::Lost("requests stopped".to_string()),
        };
        drop(requests);
        if !matches!(ended, Ended::Shutdown) {
            let _ = timeout(FINISH_SENDING, work).await;
        }
        ended
    };
    let unsent = usize::from(sending) + std::iter::from_fn(|| pending.try_recv().ok()).count();
    for _ in 0..unsent {
        offline(events);
    }
    ended
}

async fn serve_pushes<S>(
    api: &Arc<ApiService>,
    socket: &mut S,
    controls: &mut UnboundedReceiver<Control>,
    events: &UnboundedSender<SessionEvent>,
    requests: &UnboundedSender<Request>,
) -> Ended
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message>
        + Unpin,
{
    let mut keepalive = interval(KEEPALIVE);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_heard = Instant::now();
    let mut last_cluster: Option<RemoteCluster> = None;
    loop {
        tokio::select! {
            message = socket.next() => {
                last_heard = Instant::now();
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let RemotePush::Cluster(cluster) = api.parse_remote_push(&text) {
                            if last_cluster.as_ref() != Some(&cluster) {
                                last_cluster = Some(cluster.clone());
                                let _ = events.send(SessionEvent::Cluster(cluster));
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Ended::Lost("closed".to_string()),
                    Some(Err(e)) => return Ended::Lost(e.to_string()),
                    Some(Ok(_)) => {}
                }
            }
            _ = keepalive.tick() => {
                if last_heard.elapsed() > SILENCE_LIMIT {
                    return Ended::Lost("silent".to_string());
                }
                let ping = Message::Text(api.remote_keepalive().into());
                if socket.send(ping).await.is_err() {
                    return Ended::Lost("couldn't send keepalive".to_string());
                }
            }
            control = controls.recv() => match control {
                None | Some(Control::Shutdown) => return Ended::Shutdown,
                Some(Control::Reconnect) => return Ended::Reconnect,
                Some(Control::Request(request)) => {
                    let _ = requests.send(request);
                }
            },
        }
    }
}

async fn send(
    api: &ApiService,
    from: &str,
    connection_id: &str,
    request: Request,
) -> Result<(), DomainError> {
    match request {
        Request::Command { to, command } => {
            api.remote_command(from, &to, connection_id, command).await
        }
        Request::Transfer { to, playback } => {
            api.remote_transfer(from, &to, connection_id, playback)
                .await
        }
        Request::Volume { to, percent } => {
            api.remote_volume(from, &to, connection_id, percent).await
        }
    }
}
