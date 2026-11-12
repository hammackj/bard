//! `org.mpris.MediaPlayer2.bard`: what Plasma's media controls, the lock
//! screen and the media keys talk to.
//!
//! New in the Rust build. Electron got an MPRIS player implicitly from
//! Chromium's media session (as `chromium.instance<pid>`), fed from the
//! renderer's `navigator.mediaSession` -- and until 0.1.3 nothing set that
//! metadata, so the lock screen said "Bard / Bard". Here the metadata comes
//! straight from the core: title, artist, album, length and a `file://` URL
//! to the cached cover.
//!
//! State is pushed in from [`CoreEvent`]s (via [`crate::Services::handle`]);
//! D-Bus property reads are answered from that copy, never by locking
//! [`bard_core::Bard`]. Controls go out through [`AppControls`].
//!
//! [`CoreEvent`]: bard_core::CoreEvent

use crate::AppControls;
use bard_core::types::{NowPlaying, QueueItem};
use mpris_server::zbus::{self, fdo};
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Signal,
    Time, TrackId, Volume,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

pub const IDENTITY: &str = "Bard";
pub const DESKTOP_ENTRY: &str = "bard";
/// `org.mpris.MediaPlayer2.<this>`.
pub const BUS_SUFFIX: &str = "bard";

/// A jump in reported position this far from where playback should be counts
/// as a seek, and is announced with the `Seeked` signal (MPRIS clients
/// interpolate Position themselves and are only told about discontinuities).
const SEEK_TOLERANCE_SEC: f64 = 1.5;

/// The track on air, as MPRIS needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct MprisTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub path: String,
    pub duration_sec: f64,
    /// The cached cover, when the track has one.
    pub art: Option<PathBuf>,
}

impl MprisTrack {
    pub fn from_item(item: &QueueItem, artwork_dir: &Path) -> Self {
        let t = &item.track;
        MprisTrack {
            id: t.id.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            path: t.path.clone(),
            duration_sec: t.duration_sec,
            art: t
                .has_artwork
                .then(|| bard_core::artwork::artwork_path(artwork_dir, &t.id))
                .flatten(),
        }
    }
}

/// Everything the player reports.
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub track: Option<MprisTrack>,
    pub playing: bool,
    /// From the engine's progress reports; 0 until the first.
    pub duration_sec: f64,
    pub position_sec: f64,
    /// When `position_sec` was reported, to interpolate between reports.
    pub position_at: Instant,
    pub queue_length: usize,
    pub volume: f64,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            track: None,
            playing: false,
            duration_sec: 0.0,
            position_sec: 0.0,
            position_at: Instant::now(),
            queue_length: 0,
            volume: 0.8,
        }
    }
}

impl PlayerState {
    pub fn status(&self) -> PlaybackStatus {
        match (&self.track, self.playing) {
            (None, _) => PlaybackStatus::Stopped,
            (Some(_), true) => PlaybackStatus::Playing,
            (Some(_), false) => PlaybackStatus::Paused,
        }
    }

    fn length_sec(&self) -> f64 {
        let reported = self.duration_sec;
        if reported > 0.0 && reported.is_finite() {
            reported
        } else {
            self.track.as_ref().map(|t| t.duration_sec).filter(|d| *d > 0.0 && d.is_finite()).unwrap_or(0.0)
        }
    }

    /// Where playback is now, interpolated from the last report.
    pub fn position_now(&self) -> f64 {
        let mut p = self.position_sec;
        if self.playing {
            p += self.position_at.elapsed().as_secs_f64();
        }
        let len = self.length_sec();
        if len > 0.0 { p.clamp(0.0, len) } else { p.max(0.0) }
    }

    pub fn track_id(&self) -> TrackId {
        self.track
            .as_ref()
            .and_then(|t| track_id(&t.id))
            .unwrap_or(TrackId::NO_TRACK)
    }

    pub fn metadata(&self) -> Metadata {
        let mut m = Metadata::new();
        m.set_trackid(Some(self.track_id()));
        let Some(t) = &self.track else { return m };
        m.set_title(Some(t.title.clone()));
        if !t.artist.is_empty() {
            m.set_artist(Some([t.artist.clone()]));
        }
        if !t.album.is_empty() {
            m.set_album(Some(t.album.clone()));
        }
        let len = self.length_sec();
        if len > 0.0 {
            m.set_length(Some(Time::from_micros((len * 1e6).round() as i64)));
        }
        if let Some(art) = &t.art {
            m.set_art_url(Some(file_url(art)));
        }
        if !t.path.is_empty() {
            m.set_url(Some(file_url(Path::new(&t.path))));
        }
        m
    }

    pub fn can_go_next(&self) -> bool {
        // Skipping with an empty queue still has auto-fill to fall back on.
        self.track.is_some() || self.queue_length > 0
    }
    pub fn can_go_previous(&self) -> bool {
        // Previous restarts the current track when there is no history.
        self.track.is_some()
    }
    pub fn can_play(&self) -> bool {
        self.track.is_some() || self.queue_length > 0
    }
    pub fn can_pause(&self) -> bool {
        self.track.is_some()
    }
    pub fn can_seek(&self) -> bool {
        self.track.is_some() && self.length_sec() > 0.0
    }

    /// The properties a client would see, for diffing.
    fn view(&self) -> View {
        View {
            status: self.status(),
            metadata_key: self.track.clone().map(|t| (t, self.length_sec().round() as i64)),
            volume: self.volume,
            can: [self.can_go_next(), self.can_go_previous(), self.can_play(), self.can_pause(), self.can_seek()],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct View {
    status: PlaybackStatus,
    metadata_key: Option<(MprisTrack, i64)>,
    volume: f64,
    can: [bool; 5],
}

/// PropertiesChanged contents for going from `a` to `b`.
fn changed(a: &View, b: &View, state: &PlayerState) -> Vec<Property> {
    let mut out = Vec::new();
    if a.status != b.status {
        out.push(Property::PlaybackStatus(b.status));
    }
    if a.metadata_key != b.metadata_key {
        out.push(Property::Metadata(state.metadata()));
    }
    if a.volume != b.volume {
        out.push(Property::Volume(b.volume));
    }
    let [n, p, pl, pa, s] = b.can;
    if a.can[0] != n {
        out.push(Property::CanGoNext(n));
    }
    if a.can[1] != p {
        out.push(Property::CanGoPrevious(p));
    }
    if a.can[2] != pl {
        out.push(Property::CanPlay(pl));
    }
    if a.can[3] != pa {
        out.push(Property::CanPause(pa));
    }
    if a.can[4] != s {
        out.push(Property::CanSeek(s));
    }
    out
}

/// `/org/bard/track/<id>`. Track ids are hex, which is a valid path element.
pub fn track_id(id: &str) -> Option<TrackId> {
    if !bard_core::artwork::is_valid_id(id) {
        return None;
    }
    TrackId::try_from(format!("/org/bard/track/{id}")).ok()
}

/// A `file://` URL, percent-encoding everything but unreserved characters and
/// `/`, so a cover path with spaces or non-ASCII still resolves.
pub fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// What changed, from the core's events.
#[derive(Debug, Clone)]
pub enum Update {
    /// `CoreEvent::NowPlaying`.
    Track(Option<MprisTrack>),
    /// `CoreEvent::Overlay`: position, play state, duration, queue length.
    Progress(NowPlaying),
    /// `config.audio.volume`.
    Volume(f64),
}

struct Imp {
    state: Arc<Mutex<PlayerState>>,
    controls: Arc<dyn AppControls>,
}

impl Imp {
    fn state(&self) -> std::sync::MutexGuard<'_, PlayerState> {
        self.state.lock().unwrap()
    }
}

impl RootInterface for Imp {
    async fn raise(&self) -> fdo::Result<()> {
        self.controls.show_window();
        Ok(())
    }
    async fn quit(&self) -> fdo::Result<()> {
        self.controls.quit();
        Ok(())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        Ok(())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok(IDENTITY.into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(DESKTOP_ENTRY.into())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for Imp {
    async fn next(&self) -> fdo::Result<()> {
        if self.state().can_go_next() {
            self.controls.next();
        }
        Ok(())
    }
    async fn previous(&self) -> fdo::Result<()> {
        if self.state().can_go_previous() {
            self.controls.previous();
        }
        Ok(())
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.controls.pause();
        Ok(())
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.controls.toggle();
        Ok(())
    }
    async fn stop(&self) -> fdo::Result<()> {
        // Bard has no "stopped with a track loaded"; pausing is the nearest.
        self.controls.pause();
        Ok(())
    }
    async fn play(&self) -> fdo::Result<()> {
        self.controls.play();
        Ok(())
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let (target, len, can) = {
            let s = self.state();
            (s.position_now() + offset.as_micros() as f64 / 1e6, s.length_sec(), s.can_seek())
        };
        if !can {
            return Ok(());
        }
        // The spec: seeking past the end acts like Next; before the start, to 0.
        if target >= len {
            self.controls.next();
        } else {
            self.controls.seek(target.max(0.0));
        }
        Ok(())
    }
    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        let (current, len, can) = {
            let s = self.state();
            (s.track_id(), s.length_sec(), s.can_seek())
        };
        let pos = position.as_micros() as f64 / 1e6;
        // Stale requests (for a track since moved on from) and out-of-range
        // positions are ignored, as the spec says.
        if can && track_id == current && (0.0..=len).contains(&pos) {
            self.controls.seek(pos);
        }
        Ok(())
    }
    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("Bard plays its own library".into()))
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.state().status())
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(LoopStatus::None)
    }
    async fn set_loop_status(&self, _loop_status: LoopStatus) -> zbus::Result<()> {
        Err(zbus::Error::from(fdo::Error::NotSupported("loop status".into())))
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn set_rate(&self, _rate: PlaybackRate) -> zbus::Result<()> {
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_shuffle(&self, _shuffle: bool) -> zbus::Result<()> {
        Err(zbus::Error::from(fdo::Error::NotSupported("shuffle".into())))
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.state().metadata())
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.state().volume)
    }
    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        // The app applies it and saves config.audio.volume; the
        // ConfigChanged that follows updates this property.
        self.controls.set_volume(volume.clamp(0.0, 1.0));
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros((self.state().position_now() * 1e6) as i64))
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.state().can_go_next())
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.state().can_go_previous())
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self.state().can_play())
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.state().can_pause())
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.state().can_seek())
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// A running MPRIS player.
pub struct Mpris {
    state: Arc<Mutex<PlayerState>>,
    tx: mpsc::UnboundedSender<Option<f64>>,
    bus_name: String,
}

impl Mpris {
    /// Registers `org.mpris.MediaPlayer2.<suffix>`, falling back to
    /// `<suffix>.instance<pid>` when the name is taken (a second copy, or a
    /// test), as the spec allows.
    pub async fn start(
        suffix: &str,
        initial: PlayerState,
        controls: Arc<dyn AppControls>,
    ) -> zbus::Result<Mpris> {
        let state = Arc::new(Mutex::new(initial));
        let imp = || Imp { state: state.clone(), controls: controls.clone() };
        let server = match Server::new(suffix, imp()).await {
            Ok(s) => s,
            Err(_) => Server::new(&format!("{suffix}.instance{}", std::process::id()), imp()).await?,
        };
        let bus_name = server.bus_name().to_string();

        // Property changes are emitted from one task, in order. The payload is
        // Some(position) when a Seeked signal should go out as well.
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<f64>>();
        let st = state.clone();
        tokio::spawn(async move {
            let mut last = st.lock().unwrap().view();
            while let Some(seeked) = rx.recv().await {
                let (props, now) = {
                    let s = st.lock().unwrap();
                    let now = s.view();
                    (changed(&last, &now, &s), now)
                };
                last = now;
                if !props.is_empty()
                    && let Err(e) = server.properties_changed(props).await
                {
                    eprintln!("[mpris] PropertiesChanged failed: {e}");
                }
                if let Some(pos) = seeked {
                    let _ = server.emit(Signal::Seeked { position: Time::from_micros((pos * 1e6) as i64) }).await;
                }
            }
        });
        Ok(Mpris { state, tx, bus_name })
    }

    /// The bus name actually owned.
    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }

    pub fn update(&self, update: Update) {
        let seeked = {
            let mut s = self.state.lock().unwrap();
            apply(&mut s, update)
        };
        let _ = self.tx.send(seeked);
    }
}

/// Applies an update; returns the new position when it was a seek.
fn apply(s: &mut PlayerState, update: Update) -> Option<f64> {
    match update {
        Update::Track(t) => {
            if t.as_ref().map(|t| &t.id) != s.track.as_ref().map(|t| &t.id) {
                s.position_sec = 0.0;
                s.duration_sec = 0.0;
                s.position_at = Instant::now();
            }
            if t.is_none() {
                s.playing = false;
            }
            s.track = t;
            None
        }
        Update::Progress(np) => {
            let expected = s.position_now();
            let was_playing = s.playing;
            s.playing = np.is_playing;
            s.duration_sec = np.duration_sec;
            s.queue_length = np.queue_length;
            s.position_sec = np.position_sec;
            s.position_at = Instant::now();
            let jumped = s.track.is_some() && (np.position_sec - expected).abs() > SEEK_TOLERANCE_SEC;
            // A pause/resume is not a seek, even if the reports straddle it.
            (jumped && was_playing == np.is_playing).then_some(np.position_sec)
        }
        Update::Volume(v) => {
            s.volume = v;
            None
        }
    }
}

/// Starts MPRIS on `rt` from synchronous code.
pub fn start_blocking(
    rt: &Handle,
    suffix: &str,
    initial: PlayerState,
    controls: Arc<dyn AppControls>,
) -> zbus::Result<Mpris> {
    let suffix = suffix.to_owned();
    rt.block_on(async move { Mpris::start(&suffix, initial, controls).await })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> MprisTrack {
        MprisTrack {
            id: "0123456789abcdef".into(),
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            path: "/music/a b.flac".into(),
            duration_sec: 180.0,
            art: Some("/home/u/.config/bard/artwork/0123456789abcdef.jpg".into()),
        }
    }

    fn np(pos: f64, playing: bool) -> NowPlaying {
        NowPlaying {
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            duration_sec: 181.0,
            position_sec: pos,
            is_playing: playing,
            art_url: None,
            queue_length: 2,
            updated_at: String::new(),
        }
    }

    #[test]
    fn metadata_carries_the_real_track() {
        let s = PlayerState { track: Some(track()), ..Default::default() };
        let m = s.metadata();
        assert_eq!(m.title(), Some("Song"));
        assert_eq!(m.artist(), Some(vec!["Band".to_string()]));
        assert_eq!(m.album(), Some("Record"));
        assert_eq!(m.length(), Some(Time::from_secs(180)));
        assert_eq!(m.art_url().as_deref(), Some("file:///home/u/.config/bard/artwork/0123456789abcdef.jpg"));
        assert_eq!(m.url().as_deref(), Some("file:///music/a%20b.flac"));
        assert_eq!(m.trackid().unwrap().as_str(), "/org/bard/track/0123456789abcdef");
    }

    #[test]
    fn nothing_playing_is_stopped_with_no_track() {
        let s = PlayerState::default();
        assert_eq!(s.status(), PlaybackStatus::Stopped);
        assert_eq!(s.metadata().trackid(), Some(TrackId::NO_TRACK));
        assert!(!s.can_go_next() && !s.can_pause() && !s.can_seek());
    }

    #[test]
    fn progress_drives_status_and_detects_seeks() {
        let mut s = PlayerState::default();
        apply(&mut s, Update::Track(Some(track())));
        assert_eq!(apply(&mut s, Update::Progress(np(0.2, true))), None);
        assert_eq!(s.status(), PlaybackStatus::Playing);
        // Steady progress is not a seek.
        assert_eq!(apply(&mut s, Update::Progress(np(0.4, true))), None);
        // A jump is.
        assert_eq!(apply(&mut s, Update::Progress(np(90.0, true))), Some(90.0));
        // Pausing is not.
        assert_eq!(apply(&mut s, Update::Progress(np(90.0, false))), None);
        assert_eq!(s.status(), PlaybackStatus::Paused);
        // Engine duration wins over the tag's.
        assert_eq!(s.metadata().length(), Some(Time::from_secs(181)));
    }

    #[test]
    fn only_visible_changes_are_announced() {
        let mut s = PlayerState::default();
        let a = s.view();
        apply(&mut s, Update::Track(Some(track())));
        apply(&mut s, Update::Progress(np(1.0, true)));
        let b = s.view();
        let props = changed(&a, &b, &s);
        assert!(props.iter().any(|p| matches!(p, Property::Metadata(_))));
        assert!(props.iter().any(|p| matches!(p, Property::PlaybackStatus(PlaybackStatus::Playing))));
        apply(&mut s, Update::Progress(np(1.25, true)));
        assert!(changed(&b, &s.view(), &s).is_empty());
    }

    #[test]
    fn file_urls_are_percent_encoded() {
        assert_eq!(file_url(Path::new("/a/ü b#.jpg")), "file:///a/%C3%BC%20b%23.jpg");
    }
}
