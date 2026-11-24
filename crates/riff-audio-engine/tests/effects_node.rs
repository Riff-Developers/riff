use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use riff_audio_engine::{EffectsNode, ProcessorChain, EFFECTS_NODE_NAME};

fn output(program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program} runs: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every link, as (output port, input port).
fn links() -> Vec<(String, String)> {
    let mut links = vec![];
    let mut from = String::new();
    for line in output("pw-link", &["--links", "--output"]).lines() {
        match line.trim_start().strip_prefix("|-> ") {
            Some(to) => links.push((from.clone(), to.trim().to_string())),
            None => from = line.trim().to_string(),
        }
    }
    links
}

fn into_effects() -> Vec<(String, String)> {
    let input = format!("{EFFECTS_NODE_NAME}:input_");
    links()
        .into_iter()
        .filter(|(_, to)| to.starts_with(&input))
        .collect()
}

/// A stream left unconnected and named `name`, as Riff starts Soloist's.
fn stream(name: &str, silence: &std::path::Path) -> Child {
    Command::new("pw-cat")
        .args(["--playback"])
        .arg(silence)
        .env("PIPEWIRE_AUTOCONNECT", "false")
        .env("PIPEWIRE_PROPS", format!("{{ node.name = {name} }}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("pw-cat runs")
}

fn silence() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("riff-silence-{}.wav", std::process::id()));
    let data_len: u32 = 48_000 * 2 * 2 * 10;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data_len).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(48_000u32.to_le_bytes());
    wav.extend((48_000u32 * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_len.to_le_bytes());
    wav.resize(wav.len() + data_len as usize, 0);
    std::fs::write(&path, wav).expect("the silence is written");
    path
}

/// Needs a running PipeWire and its command-line tools.
#[test]
#[ignore]
fn only_the_named_stream_plays_into_the_node() {
    let source = format!("riff-test-source-{}", std::process::id());
    let node = EffectsNode::start(ProcessorChain::new(), source.clone()).expect("the node starts");
    sleep(Duration::from_millis(500));

    let nodes = output("pw-cli", &["ls", "Node"]);
    assert!(
        nodes.contains(&format!("\"{EFFECTS_NODE_NAME}\"")),
        "{EFFECTS_NODE_NAME} isn't among the nodes"
    );
    assert!(
        !output("pactl", &["list", "short", "sinks"]).contains(EFFECTS_NODE_NAME),
        "{EFFECTS_NODE_NAME} is offered as an output"
    );
    assert!(
        links()
            .iter()
            .any(|(from, _)| from.starts_with("riff-effects-output:")),
        "the effects output isn't linked"
    );
    assert!(
        into_effects().is_empty(),
        "something links into the node by itself: {:?}",
        into_effects()
    );

    let silence = silence();
    let other = format!("riff-test-other-{}", std::process::id());
    let mut others = stream(&other, &silence);
    let mut wanted = stream(&source, &silence);
    sleep(Duration::from_millis(1500));
    let linked = into_effects();
    assert_eq!(linked.len(), 2, "both channels are linked: {linked:?}");
    assert!(linked.iter().all(|(from, _)| from.starts_with(&source)));
    assert!(
        !links().iter().any(|(from, _)| from.starts_with(&other)),
        "a stream not named for it was linked"
    );

    let _ = wanted.kill();
    let _ = wanted.wait();
    sleep(Duration::from_millis(500));
    assert!(
        into_effects().is_empty(),
        "links stayed after the stream went"
    );

    let _ = others.kill();
    let _ = others.wait();
    let _ = std::fs::remove_file(&silence);
    drop(node);
    sleep(Duration::from_millis(500));
    assert!(
        !output("pw-cli", &["ls", "Node"]).contains(&format!("\"{EFFECTS_NODE_NAME}\"")),
        "{EFFECTS_NODE_NAME} stayed after it was dropped"
    );
}
