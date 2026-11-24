use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;

use crate::{AudioBuffer, ProcessorChain};

pub const NODE_NAME: &str = "riff-effects";
const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const SAMPLE: usize = std::mem::size_of::<f32>();
const MAX_HELD: usize = (RATE as usize / 2) * CHANNELS as usize;
const START_TIMEOUT: Duration = Duration::from_secs(5);

pub struct EffectsNode {
    quit: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
}

impl EffectsNode {
    pub fn start(chain: ProcessorChain, source: String) -> Result<Self, String> {
        let (quit, quit_rx) = pw::channel::channel::<()>();
        let (ready, ready_rx) = mpsc::channel::<Result<(), String>>();
        let alive = Arc::new(AtomicBool::new(true));
        let thread = std::thread::Builder::new()
            .name(NODE_NAME.to_string())
            .spawn({
                let alive = Arc::clone(&alive);
                move || {
                    if let Err(e) = run(chain, source, quit_rx, &ready) {
                        let _ = ready.send(Err(e.to_string()));
                    }
                    alive.store(false, Ordering::Relaxed);
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => {
                info!("Effects node {NODE_NAME} is up");
                Ok(Self {
                    quit,
                    thread: Some(thread),
                    alive,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // It's left to come up later: tell it to go once it does.
                let _ = quit.send(());
                Err("PipeWire didn't answer".to_string())
            }
        }
    }
}

impl EffectsNode {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn alive(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.alive)
    }
}

impl Drop for EffectsNode {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shared {
    chain: ProcessorChain,
    held: VecDeque<f32>,
    scratch: Vec<f64>,
}

struct Router {
    source: String,
    input: Option<u32>,
    source_node: Option<u32>,
    ports: HashMap<u32, Port>,
    linked: Vec<(u32, u32)>,
    links: Vec<pw::link::Link>,
}

struct Port {
    node: u32,
    output: bool,
    order: u32,
}

impl Router {
    fn new(source: String) -> Self {
        Self {
            source,
            input: None,
            source_node: None,
            ports: HashMap::new(),
            linked: Vec::new(),
            links: Vec::new(),
        }
    }

    fn ports_of(&self, node: u32, output: bool) -> Vec<u32> {
        let ports: BTreeMap<u32, u32> = self
            .ports
            .iter()
            .filter(|(_, port)| port.node == node && port.output == output)
            .map(|(&id, port)| (port.order, id))
            .collect();
        ports.into_values().collect()
    }

    fn relink(&mut self, core: &pw::core::CoreRc) {
        let wanted: Vec<(u32, u32)> = match (self.source_node, self.input) {
            (Some(source), Some(input)) => {
                let outputs = self.ports_of(source, true);
                let inputs = self.ports_of(input, false);
                if outputs.is_empty() || inputs.len() < CHANNELS as usize {
                    vec![]
                } else {
                    inputs
                        .iter()
                        .enumerate()
                        .map(|(i, &input)| (outputs[i.min(outputs.len() - 1)], input))
                        .collect()
                }
            }
            _ => vec![],
        };
        if wanted == self.linked {
            return;
        }
        self.links.clear();
        self.linked.clear();
        let (Some(source), Some(input)) = (self.source_node, self.input) else {
            return;
        };
        for (output_port, input_port) in wanted {
            let mut props = pw::properties::PropertiesBox::new();
            props.insert("link.output.node", source.to_string());
            props.insert("link.output.port", output_port.to_string());
            props.insert("link.input.node", input.to_string());
            props.insert("link.input.port", input_port.to_string());
            props.insert("object.linger", "false");
            match core.create_object::<pw::link::Link>("link-factory", &props) {
                Ok(link) => {
                    self.links.push(link);
                    self.linked.push((output_port, input_port));
                }
                Err(e) => warn!("Couldn't link {} into the effects: {e}", self.source),
            }
        }
        if !self.linked.is_empty() {
            info!("Linked {} into the effects", self.source);
        }
    }

    fn added(&mut self, global: &pw::registry::GlobalObject<&spa::utils::dict::DictRef>) -> bool {
        let Some(props) = global.props else {
            return false;
        };
        match global.type_ {
            pw::types::ObjectType::Node if props.get("node.name") == Some(self.source.as_str()) => {
                self.source_node = Some(global.id);
                true
            }
            pw::types::ObjectType::Port => {
                let node = props.get("node.id").and_then(|id| id.parse().ok());
                let order = props.get("port.id").and_then(|id| id.parse().ok());
                let (Some(node), Some(order)) = (node, order) else {
                    return false;
                };
                let output = props.get("port.direction") == Some("out");
                self.ports.insert(
                    global.id,
                    Port {
                        node,
                        output,
                        order,
                    },
                );
                Some(node) == self.source_node || Some(node) == self.input
            }
            _ => false,
        }
    }

    fn removed(&mut self, id: u32) -> bool {
        if self.source_node == Some(id) {
            self.source_node = None;
            self.ports.retain(|_, port| port.node != id);
            return true;
        }
        self.ports.remove(&id).is_some()
    }
}

fn run(
    chain: ProcessorChain,
    source: String,
    quit: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });
    let _core_listener = core
        .add_listener_local()
        .error({
            let mainloop = mainloop.clone();
            move |id, _, res, message| {
                if id == pw::core::PW_ID_CORE {
                    warn!("Lost the PipeWire connection ({res}): {message}");
                    mainloop.quit();
                }
            }
        })
        .register();

    let shared = Rc::new(RefCell::new(Shared {
        chain,
        held: VecDeque::with_capacity(MAX_HELD),
        scratch: Vec::with_capacity(MAX_HELD),
    }));

    let router = Rc::new(RefCell::new(Router::new(source)));
    let registry = core.get_registry()?;
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (router, core) = (Rc::clone(&router), core.clone());
            move |global| {
                let mut router = router.borrow_mut();
                if router.added(global) {
                    router.relink(&core);
                }
            }
        })
        .global_remove({
            let (router, core) = (Rc::clone(&router), core.clone());
            move |id| {
                let mut router = router.borrow_mut();
                if router.removed(id) {
                    router.relink(&core);
                }
            }
        })
        .register();

    let sink = pw::stream::StreamBox::new(
        &core,
        NODE_NAME,
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::NODE_NAME => NODE_NAME,
            *pw::keys::NODE_DESCRIPTION => "Riff Effects",
            *pw::keys::NODE_GROUP => NODE_NAME,
            *pw::keys::NODE_LINK_GROUP => NODE_NAME,
            *pw::keys::NODE_AUTOCONNECT => "false",
            "node.dont-reconnect" => "true",
            "audio.position" => "FL,FR",
        },
    )?;
    let _sink_listener = sink
        .add_local_listener_with_user_data(Rc::clone(&shared))
        .state_changed({
            let (router, core) = (Rc::clone(&router), core.clone());
            let mut on_error = quit_on_error(&mainloop);
            move |stream, shared, old, new| {
                if matches!(
                    new,
                    pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming
                ) {
                    let mut router = router.borrow_mut();
                    let id = stream.node_id();
                    if router.input != Some(id) {
                        router.input = Some(id);
                        router.relink(&core);
                    }
                }
                on_error(stream, shared, old, new);
            }
        })
        .process(|stream, shared| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let offset = data.chunk().offset() as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let Some(bytes) = bytes.get(offset..offset + size) else {
                return;
            };
            let mut shared = shared.borrow_mut();
            let mut samples = std::mem::take(&mut shared.scratch);
            samples.clear();
            samples.extend(
                bytes
                    .chunks_exact(SAMPLE)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
            );
            let mut audio = AudioBuffer::new(samples, CHANNELS as u16, RATE);
            shared.chain.process(&mut audio);
            let incoming = audio.samples.len().min(MAX_HELD);
            let over = (shared.held.len() + incoming).saturating_sub(MAX_HELD);
            shared.held.drain(..over);
            let skip = audio.samples.len() - incoming;
            shared
                .held
                .extend(audio.samples[skip..].iter().map(|&s| s as f32));
            shared.scratch = audio.samples;
        })
        .register()?;

    let output = pw::stream::StreamBox::new(
        &core,
        "riff-effects-output",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::NODE_NAME => "riff-effects-output",
            *pw::keys::NODE_DESCRIPTION => "Riff Effects Output",
            *pw::keys::NODE_GROUP => NODE_NAME,
            *pw::keys::NODE_LINK_GROUP => NODE_NAME,
            "node.dont-reconnect" => "false",
            "audio.position" => "FL,FR",
        },
    )?;
    let _output_listener = output
        .add_local_listener_with_user_data(Rc::clone(&shared))
        .state_changed(quit_on_error(&mainloop))
        .process(|stream, shared| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let stride = SAMPLE * CHANNELS as usize;
            let written = match data.data() {
                Some(bytes) => {
                    let room = bytes.len() / stride;
                    let frames = if requested > 0 {
                        requested.min(room)
                    } else {
                        room
                    };
                    let mut shared = shared.borrow_mut();
                    for out in bytes[..frames * stride].chunks_exact_mut(SAMPLE) {
                        let sample = shared.held.pop_front().unwrap_or(0.0);
                        out.copy_from_slice(&sample.to_le_bytes());
                    }
                    frames
                }
                None => 0,
            };
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as _;
            *chunk.size_mut() = (stride * written) as _;
        })
        .register()?;

    let format = format_param();
    let mut sink_params = [Pod::from_bytes(&format).expect("a valid format")];
    sink.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS,
        &mut sink_params,
    )?;
    let mut output_params = [Pod::from_bytes(&format).expect("a valid format")];
    output.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut output_params,
    )?;

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}

fn quit_on_error<D>(
    mainloop: &pw::main_loop::MainLoopRc,
) -> impl FnMut(&pw::stream::Stream, &mut D, pw::stream::StreamState, pw::stream::StreamState) {
    let mainloop = mainloop.clone();
    move |_, _, _, state| {
        if let pw::stream::StreamState::Error(e) = state {
            warn!("An effects stream failed: {e}");
            mainloop.quit();
        }
    }
}

fn format_param() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(CHANNELS);
    let mut position = [0; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .expect("a format serializes")
    .0
    .into_inner()
}
