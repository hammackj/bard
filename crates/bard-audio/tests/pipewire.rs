//! Against the real PipeWire daemon, inaudibly: the player runs in silent
//! mode (the device is sent zeros) *and* at volume 0. Checks with `pw-dump`
//! that the stream appears, is linked to the requested sink, and moves when
//! the output changes, without losing its place.
//!
//! Needs a running PipeWire session, so it is opt-in:
//!     cargo test -p bard-audio --test pipewire -- --ignored --nocapture

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use bard_audio::{Backend, Player, PlayerEvent, PlayerOptions};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn dump() -> Vec<Value> {
    let out = Command::new("pw-dump").output().expect("pw-dump");
    serde_json::from_slice(&out.stdout).expect("pw-dump json")
}

fn props(o: &Value) -> &Value {
    &o["info"]["props"]
}

/// node.names of the sinks our node is linked to, and our node's target.object.
fn routing(node_name: &str) -> Option<(Vec<String>, Option<String>)> {
    let objs = dump();
    let node = objs.iter().find(|o| o["type"] == "PipeWire:Interface:Node" && props(o)["node.name"] == node_name)?;
    let id = node["id"].as_u64()?;
    let target = props(node)["target.object"].as_str().map(str::to_string);
    let mut sinks: Vec<String> = objs
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Link" && o["info"]["output-node-id"].as_u64() == Some(id))
        .filter_map(|l| {
            let to = l["info"]["input-node-id"].as_u64()?;
            let n = objs.iter().find(|o| o["id"].as_u64() == Some(to))?;
            props(n)["node.name"].as_str().map(str::to_string)
        })
        .collect();
    sinks.sort();
    sinks.dedup();
    Some((sinks, target))
}

fn default_sink() -> Option<String> {
    let objs = dump();
    let meta =
        objs.iter().find(|o| o["type"] == "PipeWire:Interface:Metadata" && o["props"]["metadata.name"] == "default")?;
    let entry = meta["metadata"].as_array()?.iter().find(|m| m["key"] == "default.audio.sink")?;
    entry["value"]["name"].as_str().map(str::to_string)
}

fn wait_routed(node: &str, want: &str) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let (sinks, _) = routing(node).unwrap_or_default();
        if sinks.iter().any(|s| s == want) || Instant::now() > end {
            return sinks;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn drain(rx: &Receiver<PlayerEvent>) -> Vec<PlayerEvent> {
    rx.try_iter().collect()
}

#[test]
#[ignore = "needs a PipeWire session; silent, but touches the real graph"]
fn stream_appears_targets_and_moves_silently() {
    let node = format!("bard-audio-selftest-{}", std::process::id());
    let (p, rx) = Player::with_options(PlayerOptions {
        backend: Backend::PipeWire,
        silent: true,
        node_name: node.clone(),
        app_name: "bard-audio selftest".into(),
    })
    .expect("PipeWire player");
    // Belt and braces: silent mode already sends zeros.
    p.set_volume(0.0);

    let outputs = p.outputs();
    eprintln!("sinks:");
    for o in &outputs {
        eprintln!("  {:<60} {}", o.id, o.name);
    }
    assert!(!outputs.is_empty(), "registry reported no sinks");
    let default = default_sink().expect("default sink in metadata");
    eprintln!("default sink: {default}");

    // Inactive before anything plays, but the node exists.
    assert!(routing(&node).is_some(), "stream node exists");

    p.play("t".into(), fixture("bursts.flac"), 1.0, 0.0);
    let sinks = wait_routed(&node, &default);
    eprintln!("playing -> {sinks:?}");
    assert!(sinks.contains(&default), "linked to the default sink: {sinks:?}");
    std::thread::sleep(Duration::from_millis(400));
    let ev = drain(&rx);
    assert!(
        ev.iter().any(|e| matches!(e, PlayerEvent::Progress { playing: true, track_id: Some(t), .. } if t == "t")),
        "progress while playing: {ev:?}"
    );
    assert!(ev.iter().all(|e| !matches!(e, PlayerEvent::Error(_))), "{ev:?}");

    // Move to another sink mid-track. A virtual sink is preferred (or set
    // BARD_TEST_SINK): moving onto a suspended hardware sink wakes it, which
    // some hardware answers with a pop even when fed silence.
    let other = std::env::var("BARD_TEST_SINK").ok().unwrap_or_else(|| {
        let mut c: Vec<_> = outputs.iter().filter(|o| o.id != default).collect();
        c.sort_by_key(|o| o.id.starts_with("alsa_"));
        c.first().expect("a second sink").id.clone()
    });
    let before = p.position().position;
    p.set_output(&other).expect("known sink");
    let sinks = wait_routed(&node, &other);
    eprintln!("set_output({other}) -> {sinks:?}");
    assert_eq!(sinks, vec![other.clone()], "moved to the requested sink");
    assert_eq!(routing(&node).unwrap().1.as_deref(), Some(other.as_str()), "target.object set");
    let after = p.position();
    eprintln!("position {before:.3} -> {:.3}", after.position);
    assert!(after.playing && after.position >= before, "kept playing from the same place");

    // Back to the default.
    p.set_output("default").unwrap();
    let sinks = wait_routed(&node, &default);
    eprintln!("set_output(default) -> {sinks:?} target {:?}", routing(&node).unwrap().1);
    assert!(sinks.contains(&default));
    assert_eq!(routing(&node).unwrap().1, None, "target.object removed");

    // Unknown id: falls back to the default and says so.
    p.set_output(&other).unwrap();
    wait_routed(&node, &other);
    assert!(p.set_output("alsa_output.does-not-exist").is_err());
    let sinks = wait_routed(&node, &default);
    eprintln!("set_output(unknown) -> {sinks:?}");
    assert!(sinks.contains(&default));
    let ev = drain(&rx);
    assert!(ev.iter().any(|e| matches!(e, PlayerEvent::Error(m) if m.contains("using system default"))), "{ev:?}");

    // Plays out to the end across all that.
    let end = Instant::now() + Duration::from_secs(5);
    let mut ended = false;
    while Instant::now() < end && !ended {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(100)) {
            ended = e == PlayerEvent::Ended;
        }
    }
    assert!(ended, "Ended arrived");
    eprintln!("ended at {:.3}s", p.position().position);
    assert!(p.analyser().fft_size() == 2048);

    drop(p);
    std::thread::sleep(Duration::from_millis(300));
    assert!(routing(&node).is_none(), "stream gone after drop");
}
