//! Output through the OS audio API, for macOS (CoreAudio) and Windows
//! (WASAPI), by way of cpal. Linux uses PipeWire directly (`pw.rs`).
//!
//! One thread owns the stream and does everything that may block: building
//! and rebuilding streams, and polling the device list. The device's callback
//! only renders.
//!
//! The engine renders stereo f32 at [`OUT_RATE`]. The stream is opened at the
//! device's own rate, channel count and sample format, and converted here,
//! rather than asking for 48 kHz: on macOS that request changes the device's
//! nominal rate for every app using it, which on a streaming setup's audio
//! interface is exactly what must not happen.
//!
//! Routing mirrors the PipeWire output: `None` is the system default (and
//! follows it as it changes), a device id pins one device. A pinned device
//! that disappears falls back to the default, and playback moves back on its
//! own when the device returns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::decode::{CHANNELS, OUT_RATE};
use crate::engine::Output;
use crate::render::{Renderer, Shared};
use crate::{OutputDevice, PlayerEvent};

/// How often the device list is re-read.
const POLL: Duration = Duration::from_secs(2);

/// Frames rendered at a time. 10 ms at 48 kHz.
const BLOCK: usize = 480;

enum Cmd {
    SetActive(bool),
    SetTarget(Option<String>),
    /// The stream's device went away.
    Lost,
    Quit,
}

pub(crate) struct NativeOutput {
    tx: Sender<Cmd>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NativeOutput {
    pub fn spawn(renderer: Renderer, shared: Arc<Shared>, events: Sender<PlayerEvent>) -> Result<NativeOutput> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let lost = tx.clone();
        let thread = std::thread::Builder::new()
            .name("bard-audio-out".into())
            .spawn(move || {
                let mut out = Router::new(renderer, shared, events, lost);
                // The device list is ready before `Player::new` returns; the
                // stream is opened after, so the caller (the UI thread, at
                // startup) never waits on the audio device. On macOS that
                // wait can involve the main run loop, which is the caller.
                out.publish();
                let _ = ready_tx.send(Ok(()));
                out.rebuild();
                out.run(rx);
            })
            .context("spawning the audio output thread")?;
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("the audio output did not start within 5 s"))?
            .map_err(|e| anyhow!("audio output: {e}"))?;
        Ok(NativeOutput { tx, thread: Some(thread) })
    }
}

impl Output for NativeOutput {
    fn set_active(&mut self, active: bool) {
        let _ = self.tx.send(Cmd::SetActive(active));
    }
    fn set_target(&mut self, target: Option<String>) {
        let _ = self.tx.send(Cmd::SetTarget(target));
    }
}

impl Drop for NativeOutput {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

/// Owns the stream; lives on the output thread.
struct Router {
    host: cpal::Host,
    /// Shared with whichever stream is current. Only one stream exists at a
    /// time (the old one is dropped, which stops its callback, before the
    /// next is built), so the callback's `try_lock` never actually contends.
    renderer: Arc<Mutex<Renderer>>,
    shared: Arc<Shared>,
    events: Sender<PlayerEvent>,
    lost: Sender<Cmd>,
    active: bool,
    /// What was asked for: `None` is the system default.
    target: Option<String>,
    /// The device the stream is on, `None` when it is on the default.
    on: Option<String>,
    /// The id of the device the stream was opened on, default or not: a
    /// default that has since changed is followed by reopening.
    opened: Option<String>,
    stream: Option<cpal::Stream>,
    outputs: Vec<OutputDevice>,
    /// The last error reported, so a device that keeps failing to open is
    /// reported once rather than on every poll.
    last_error: Option<String>,
}

impl Router {
    fn new(renderer: Renderer, shared: Arc<Shared>, events: Sender<PlayerEvent>, lost: Sender<Cmd>) -> Router {
        Router {
            host: cpal::default_host(),
            renderer: Arc::new(Mutex::new(renderer)),
            shared,
            events,
            lost,
            active: false,
            target: None,
            on: None,
            opened: None,
            stream: None,
            outputs: Vec::new(),
            last_error: None,
        }
    }

    fn error(&mut self, msg: impl std::fmt::Display) {
        let msg = format!("audio output: {msg}");
        if self.last_error.as_ref() != Some(&msg) {
            let _ = self.events.send(PlayerEvent::Error(msg.clone()));
            self.last_error = Some(msg);
        }
    }

    fn run(&mut self, rx: mpsc::Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(POLL) {
                Ok(Cmd::SetActive(on)) => {
                    self.active = on;
                    if self.stream.is_none() && on {
                        self.rebuild();
                    } else {
                        self.apply_active();
                    }
                }
                Ok(Cmd::SetTarget(target)) => {
                    self.target = target;
                    self.rebuild();
                }
                Ok(Cmd::Lost) => {
                    self.stream = None;
                    self.publish();
                    self.rebuild();
                }
                Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    self.publish();
                    let back = self
                        .target
                        .as_ref()
                        .is_some_and(|t| self.on.as_ref() != Some(t) && self.outputs.iter().any(|o| &o.id == t));
                    // Not every backend moves a default-device stream when the
                    // system default changes; reopen on the new one if not.
                    let moved = self.target.is_none()
                        && self.stream.is_some()
                        && self.host.default_output_device().and_then(|d| device_id(&d)) != self.opened;
                    if back || moved || self.stream.is_none() {
                        self.rebuild();
                    }
                }
            }
        }
        self.stream = None;
    }

    /// Re-reads the output devices into `shared.outputs`.
    fn publish(&mut self) {
        let mut list: Vec<OutputDevice> = match self.host.output_devices() {
            Ok(devices) => devices
                .filter_map(|d| {
                    let id = device_id(&d)?;
                    let name = d.description().map(|n| n.name().to_string()).unwrap_or_else(|_| id.clone());
                    Some(OutputDevice { id, name })
                })
                .collect(),
            Err(_) => return,
        };
        list.sort_by_key(|o| o.name.to_lowercase());
        if list != self.outputs {
            self.outputs = list.clone();
            if let Ok(mut out) = self.shared.outputs.lock() {
                *out = list;
            }
        }
    }

    fn apply_active(&mut self) {
        let Some(stream) = &self.stream else { return };
        let res = if self.active { stream.play() } else { stream.pause() };
        if let Err(e) = res {
            self.error(e);
        }
    }

    /// Opens a stream on the target if it is there, otherwise the default.
    fn rebuild(&mut self) {
        // Dropped first: its callback holds the renderer.
        self.stream = None;
        let pinned = self
            .target
            .as_ref()
            .and_then(|t| self.host.output_devices().ok()?.find(|d| device_id(d).as_ref() == Some(t)));
        let (device, on) = match pinned {
            Some(d) => {
                let id = device_id(&d);
                (Some(d), id)
            }
            None => (self.host.default_output_device(), None),
        };
        let Some(device) = device else {
            // No device at all; the poll tries again.
            self.on = None;
            self.opened = None;
            return;
        };
        match self.open(&device) {
            Ok(stream) => {
                self.stream = Some(stream);
                self.on = on;
                self.opened = device_id(&device);
                self.last_error = None;
                self.apply_active();
            }
            Err(e) => {
                self.on = None;
                self.error(format!("{e:#}"));
            }
        }
    }

    fn open(&self, device: &cpal::Device) -> Result<cpal::Stream> {
        let supported = device.default_output_config().context("reading the device's format")?;
        let config = supported.config();
        let format = supported.sample_format();
        let feed = Feed::new(self.renderer.clone(), config.sample_rate, config.channels as usize)?;
        match format {
            SampleFormat::F32 => self.build::<f32>(device, &config, feed),
            SampleFormat::I16 => self.build::<i16>(device, &config, feed),
            SampleFormat::I32 => self.build::<i32>(device, &config, feed),
            SampleFormat::U16 => self.build::<u16>(device, &config, feed),
            SampleFormat::F64 => self.build::<f64>(device, &config, feed),
            other => Err(anyhow!("unsupported device sample format {other:?}")),
        }
    }

    fn build<T>(&self, device: &cpal::Device, config: &cpal::StreamConfig, mut feed: Feed) -> Result<cpal::Stream>
    where
        T: SizedSample + FromSample<f32>,
    {
        let lost = self.lost.clone();
        let events = self.events.clone();
        let gone = Arc::new(AtomicBool::new(false));
        let stream = device
            .build_output_stream::<T, _, _>(
                *config,
                move |data: &mut [T], _| feed.fill(data),
                move |err: cpal::Error| match err.kind() {
                    // The default device changed and the stream followed it.
                    cpal::ErrorKind::DeviceChanged => {}
                    cpal::ErrorKind::DeviceNotAvailable => {
                        if !gone.swap(true, Ordering::Relaxed) {
                            let _ = lost.send(Cmd::Lost);
                        }
                    }
                    _ => {
                        let _ = events.send(PlayerEvent::Error(format!("audio output: {err}")));
                    }
                },
                None,
            )
            .context("opening the output stream")?;
        Ok(stream)
    }
}

/// The callback's side: renders, converts the rate if the device is not at
/// 48 kHz, and lays the stereo frames out in the device's channels and
/// sample format. Allocates nothing once built.
struct Feed {
    renderer: Arc<Mutex<Renderer>>,
    channels: usize,
    /// Rendered stereo at [`OUT_RATE`], one block.
    block: Vec<f32>,
    resample: Option<OutResample>,
    /// Stereo frames ready for the device, and how far into them.
    ready: Vec<f32>,
    pos: usize,
}

struct OutResample {
    rs: Fft<f32>,
    out: Vec<f32>,
}

impl Feed {
    fn new(renderer: Arc<Mutex<Renderer>>, rate: u32, channels: usize) -> Result<Feed> {
        let (resample, ready) = if rate != OUT_RATE {
            let rs = Fft::<f32>::new(OUT_RATE as usize, rate as usize, BLOCK, CHANNELS, FixedSync::Input)
                .map_err(|e| anyhow!("resampler {OUT_RATE} -> {rate}: {e}"))?;
            let out = vec![0.0; rs.output_frames_max() * CHANNELS];
            let ready = Vec::with_capacity(out.len());
            (Some(OutResample { rs, out }), ready)
        } else {
            (None, vec![0.0; BLOCK * CHANNELS])
        };
        // Starts used up, so the first callback renders.
        let pos = ready.len();
        Ok(Feed { renderer, channels: channels.max(1), block: vec![0.0; BLOCK * CHANNELS], resample, ready, pos })
    }

    /// Renders the next block into `ready`.
    fn refill(&mut self) {
        match self.renderer.try_lock() {
            Ok(mut r) => r.render(&mut self.block),
            Err(_) => self.block.fill(0.0),
        }
        self.pos = 0;
        let Some(rs) = &mut self.resample else {
            // Both are one block long: swap rather than copy.
            std::mem::swap(&mut self.ready, &mut self.block);
            return;
        };
        let max = rs.rs.output_frames_max();
        let produced = (|| {
            let inp = InterleavedSlice::new(&self.block[..], CHANNELS, BLOCK).ok()?;
            let mut outp = InterleavedSlice::new_mut(&mut rs.out[..], CHANNELS, max).ok()?;
            rs.rs.process_into_buffer(&inp, &mut outp, None).ok().map(|(_, n)| n)
        })()
        .unwrap_or(0);
        self.ready.clear();
        self.ready.extend_from_slice(&rs.out[..produced * CHANNELS]);
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, data: &mut [T]) {
        for frame in data.chunks_mut(self.channels) {
            if self.pos >= self.ready.len() {
                self.refill();
                if self.ready.is_empty() {
                    frame.fill(T::EQUILIBRIUM);
                    continue;
                }
            }
            let (l, r) = (self.ready[self.pos], self.ready[self.pos + 1]);
            self.pos += CHANNELS;
            match frame {
                [mono] => *mono = T::from_sample((l + r) * 0.5),
                [left, right, rest @ ..] => {
                    *left = T::from_sample(l);
                    *right = T::from_sample(r);
                    rest.fill(T::EQUILIBRIUM);
                }
                [] => {}
            }
        }
    }
}
