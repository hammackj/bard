//! PipeWire output: one playback stream, plus a registry watch for sinks.
//!
//! Runs its own PipeWire main loop on a dedicated thread. The stream's
//! process callback runs on PipeWire's realtime data thread (`RT_PROCESS`)
//! and only calls [`Renderer::render`], which is lock-free.
//!
//! Routing is by `target.object` = the sink's `node.name`, which is stable
//! across reboots and replugs, unlike the numeric node id. Moving to another
//! sink disconnects and reconnects the same stream with the new target: the
//! renderer, its queued audio and the position all live outside PipeWire, so
//! playback carries on from where it was, after a gap of a few milliseconds.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::{CString, c_char};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use pw::stream::{StreamFlags, StreamRc, StreamState};
use pw::types::ObjectType;

use crate::decode::{CHANNELS, OUT_RATE};
use crate::engine::Output;
use crate::render::{Renderer, Shared};
use crate::{OutputDevice, PlayerEvent};

enum PwCmd {
    SetActive(bool),
    SetTarget(Option<String>),
    Quit,
}

pub(crate) struct PwOutput {
    tx: pw::channel::Sender<PwCmd>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct PwOptions {
    pub node_name: String,
    pub app_name: String,
}

impl PwOutput {
    pub fn spawn(
        renderer: Renderer,
        shared: Arc<Shared>,
        events: Sender<PlayerEvent>,
        opts: PwOptions,
    ) -> Result<PwOutput> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<pw::channel::Sender<PwCmd>, String>>();
        let thread = std::thread::Builder::new()
            .name("bard-pipewire".into())
            .spawn(move || {
                let fail = ready_tx.clone();
                if let Err(e) = run(renderer, shared, events, opts, ready_tx) {
                    let _ = fail.send(Err(format!("{e:#}")));
                }
            })
            .context("spawning the PipeWire thread")?;
        let tx = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("PipeWire did not answer within 5 s"))?
            .map_err(|e| anyhow!("PipeWire: {e}"))?;
        Ok(PwOutput { tx, thread: Some(thread) })
    }
}

impl Output for PwOutput {
    fn set_active(&mut self, active: bool) {
        let _ = self.tx.send(PwCmd::SetActive(active));
    }
    fn set_target(&mut self, target: Option<String>) {
        let _ = self.tx.send(PwCmd::SetTarget(target));
    }
}

impl Drop for PwOutput {
    fn drop(&mut self) {
        let _ = self.tx.send(PwCmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The stream's one format: interleaved stereo f32 at 48 kHz. PipeWire
/// converts to whatever the sink runs at.
fn format_pod() -> Result<Vec<u8>> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(OUT_RATE);
    info.set_channels(CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .map_err(|e| anyhow!("serialising the stream format: {e:?}"))?
    .0
    .into_inner();
    Ok(bytes)
}

/// Sets or (with `None`) removes `target.object` on the stream's properties.
/// The safe wrapper has no way to remove a key, so this goes to the C API.
fn set_target_prop(stream: &StreamRc, target: Option<&str>) {
    let key = c"target.object";
    let value = target.and_then(|t| CString::new(t).ok());
    let item = spa::sys::spa_dict_item {
        key: key.as_ptr(),
        value: value.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()) as *const c_char,
    };
    let dict = spa::sys::spa_dict { flags: 0, n_items: 1, items: &item };
    // A NULL value removes the key (pw_properties_update).
    unsafe {
        pw::sys::pw_stream_update_properties(stream.as_raw_ptr(), &dict);
    }
}

fn connect(stream: &StreamRc, format: &[u8], active: bool) -> Result<()> {
    let pod = Pod::from_bytes(format).ok_or_else(|| anyhow!("bad format pod"))?;
    let mut params = [pod];
    let mut flags = StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;
    if !active {
        flags |= StreamFlags::INACTIVE;
    }
    stream.connect(spa::utils::Direction::Output, None, flags, &mut params)?;
    Ok(())
}

fn run(
    renderer: Renderer,
    shared: Arc<Shared>,
    events: Sender<PlayerEvent>,
    opts: PwOptions,
    ready: Sender<Result<pw::channel::Sender<PwCmd>, String>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None).context("connecting to the PipeWire daemon")?;
    let registry = core.get_registry_rc()?;

    // Sinks, by global id, mirrored into `shared.outputs` on every change.
    let sinks: Rc<RefCell<BTreeMap<u32, OutputDevice>>> = Rc::default();
    let publish = {
        let shared = shared.clone();
        let sinks = sinks.clone();
        move || {
            let mut list: Vec<OutputDevice> = sinks.borrow().values().cloned().collect();
            list.sort_by_key(|o| o.name.to_lowercase());
            if let Ok(mut out) = shared.outputs.lock() {
                *out = list;
            }
        }
    };
    let _reg_listener = registry
        .add_listener_local()
        .global({
            let sinks = sinks.clone();
            let publish = publish.clone();
            move |g| {
                if g.type_ != ObjectType::Node {
                    return;
                }
                let Some(props) = g.props else { return };
                if props.get("media.class") != Some("Audio/Sink") {
                    return;
                }
                let Some(id) = props.get("node.name") else { return };
                let name = props
                    .get("node.description")
                    .or_else(|| props.get("node.nick"))
                    .unwrap_or(id);
                sinks.borrow_mut().insert(g.id, OutputDevice { id: id.to_string(), name: name.to_string() });
                publish();
            }
        })
        .global_remove({
            let sinks = sinks.clone();
            move |id| {
                if sinks.borrow_mut().remove(&id).is_some() {
                    publish();
                }
            }
        })
        .register();

    let stream = StreamRc::new(
        core.clone(),
        &opts.app_name,
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::MEDIA_ROLE => "Music",
            *pw::keys::MEDIA_NAME => opts.app_name.as_str(),
            *pw::keys::APP_NAME => opts.app_name.as_str(),
            *pw::keys::APP_ICON_NAME => "bard",
            *pw::keys::NODE_NAME => opts.node_name.as_str(),
            *pw::keys::NODE_DESCRIPTION => opts.app_name.as_str(),
            *pw::keys::NODE_LATENCY => "1024/48000",
            *pw::keys::AUDIO_CHANNELS => "2",
        },
    )?;

    let _process = stream
        .add_local_listener_with_user_data(renderer)
        .process(|stream, renderer| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else { return };
            let stride = CHANNELS * std::mem::size_of::<f32>();
            let mut frames = 0;
            if let Some(bytes) = data.data() {
                // SAFETY: f32 has no invalid bit patterns; the prefix check
                // rejects a misaligned mapping.
                let (pre, samples, _) = unsafe { bytes.align_to_mut::<f32>() };
                if pre.is_empty() {
                    let max = samples.len() / CHANNELS;
                    frames = if requested > 0 { requested.min(max) } else { max };
                    renderer.render(&mut samples[..frames * CHANNELS]);
                }
            }
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = (frames * stride) as u32;
        })
        .register()?;

    let _state = stream
        .add_local_listener::<()>()
        .state_changed({
            let events = events.clone();
            move |_, _, _old, new| {
                if let StreamState::Error(msg) = new {
                    let _ = events.send(PlayerEvent::Error(format!("audio output: {msg}")));
                }
            }
        })
        .register()?;

    let format = format_pod()?;
    connect(&stream, &format, false)?;

    let active = Rc::new(Cell::new(false));
    let (tx, rx) = pw::channel::channel::<PwCmd>();
    let _rx = rx.attach(mainloop.loop_(), {
        let stream = stream.clone();
        let mainloop = mainloop.clone();
        let events = events.clone();
        move |cmd| match cmd {
            PwCmd::SetActive(on) => {
                active.set(on);
                if let Err(e) = stream.set_active(on) {
                    let _ = events.send(PlayerEvent::Error(format!("audio output: {e}")));
                }
            }
            PwCmd::SetTarget(target) => {
                set_target_prop(&stream, target.as_deref());
                let res = stream
                    .disconnect()
                    .map_err(anyhow::Error::from)
                    .and_then(|_| connect(&stream, &format, active.get()));
                if let Err(e) = res {
                    let _ = events.send(PlayerEvent::Error(format!("audio output: {e:#}")));
                }
            }
            PwCmd::Quit => mainloop.quit(),
        }
    });

    // Answer the caller once the registry has delivered the existing sinks,
    // so `outputs()` is complete as soon as `Player::new` returns.
    let pending = core.sync(0)?;
    let _core_listener = core
        .add_listener_local()
        .done({
            let ready = RefCell::new(Some((ready, tx)));
            move |id, seq| {
                if id == pw::core::PW_ID_CORE
                    && seq == pending
                    && let Some((ready, tx)) = ready.borrow_mut().take()
                {
                    let _ = ready.send(Ok(tx));
                }
            }
        })
        .error({
            let events = events.clone();
            move |_, _, res, msg| {
                let _ = events.send(PlayerEvent::Error(format!("PipeWire error {res}: {msg}")));
            }
        })
        .register();

    mainloop.run();
    let _ = stream.disconnect();
    Ok(())
}
