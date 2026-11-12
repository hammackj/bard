//! The OBS browser-source overlay: static page, `/nowplaying.json`, SSE and
//! cover art. A port of `src/main/overlay.ts` (express + SSE).
//!
//! `resources/overlay/` is embedded in the binary unchanged, so the server
//! works from any install location.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bard_core::types::NowPlaying;
use bard_core::util::{Indent, to_js_json};
use include_dir::{Dir, include_dir};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

static OVERLAY: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../resources/overlay");

/// If the configured port is taken, walk upward this many times before failing.
pub const PORT_ATTEMPTS: u16 = 10;

/// Binds to 127.0.0.1 explicitly, walking upward from `start` while the port
/// is in use.
///
/// Never 0.0.0.0: on a shared or public network that would expose the overlay,
/// and the /art route, to anyone who can reach the machine.
pub fn bind_with_retry(start: u16) -> Option<TcpListener> {
    let mut port = start;
    for attempt in 0..PORT_ATTEMPTS {
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
            Ok(l) => return Some(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempt < PORT_ATTEMPTS - 1 => {
                port = port.checked_add(1)?;
            }
            Err(_) => return None,
        }
    }
    None
}

/// A frame as the overlay page parses it: `data: <JSON>\n\n`, the JSON
/// written the way `JSON.stringify` would (whole numbers without `.0`).
fn frame(np: &NowPlaying) -> String {
    format!("data: {}\n\n", json(np))
}

fn json(np: &NowPlaying) -> String {
    to_js_json(np, Indent::Compact).unwrap_or_else(|_| "{}".into())
}

struct Shared {
    artwork_dir: PathBuf,
    /// The latest snapshot, for /nowplaying.json and a new client's first frame.
    latest: Mutex<NowPlaying>,
    frames: broadcast::Sender<Arc<str>>,
    clients: AtomicUsize,
}

/// A running overlay server. Dropping it stops it and frees the port.
pub struct OverlayServer {
    port: u16,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl OverlayServer {
    /// Binds (see [`bind_with_retry`]) and serves on `rt`. None when no port
    /// in range could be bound.
    ///
    /// `initial` is what `/nowplaying.json` serves until the first [`push`].
    ///
    /// [`push`]: OverlayServer::push
    pub fn start(rt: &Handle, preferred_port: u16, artwork_dir: PathBuf, initial: NowPlaying) -> Option<Self> {
        let std_listener = bind_with_retry(preferred_port)?;
        let port = std_listener.local_addr().ok()?.port();
        std_listener.set_nonblocking(true).ok()?;
        let listener = {
            let _guard = rt.enter();
            tokio::net::TcpListener::from_std(std_listener).ok()?
        };
        let (frames, _) = broadcast::channel(64);
        let shared =
            Arc::new(Shared { artwork_dir, latest: Mutex::new(initial), frames, clients: AtomicUsize::new(0) });
        let app = router(shared.clone());
        let task = rt.spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("[overlay] server stopped: {e}");
            }
        });
        Some(OverlayServer { port, shared, task })
    }

    /// The port actually bound -- what [`bard_core::Bard::set_overlay_port`]
    /// needs, since cover art URLs are built from it.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Pushes a snapshot to every SSE client (`onNowPlayingChange`). Call for
    /// every [`bard_core::CoreEvent::Overlay`].
    pub fn push(&self, np: &NowPlaying) {
        *self.shared.latest.lock().unwrap() = np.clone();
        // No receivers is not an error: nobody is watching.
        let _ = self.shared.frames.send(frame(np).into());
    }

    pub fn client_count(&self) -> usize {
        self.shared.clients.load(Ordering::Relaxed)
    }
}

impl Drop for OverlayServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/", get(|| async { static_file("index.html") }))
        .route("/nowplaying.json", get(now_playing))
        .route("/events", get(events))
        .route("/art/{track_id}", get(art))
        .fallback(get(|uri: axum::http::Uri| async move { static_file(uri.path().trim_start_matches('/')) }))
        .with_state(shared)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=UTF-8",
        "css" => "text/css; charset=UTF-8",
        "js" => "application/javascript; charset=UTF-8",
        "json" => "application/json; charset=UTF-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// express.static with `maxAge: 0`. Files come from the embedded directory,
/// so nothing outside it can be reached however the path is spelled.
fn static_file(path: &str) -> Response {
    let path = if path.is_empty() || path.ends_with('/') { format!("{path}index.html") } else { path.into() };
    match OVERLAY.get_file(&path) {
        Some(f) => {
            ([(header::CONTENT_TYPE, content_type(&path)), (header::CACHE_CONTROL, "public, max-age=0")], f.contents())
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn now_playing(State(s): State<Arc<Shared>>) -> Response {
    let body = json(&s.latest.lock().unwrap());
    ([(header::CONTENT_TYPE, "application/json; charset=utf-8")], body).into_response()
}

/// Artwork is served from the on-disk cache written during the library scan.
///
/// The id stays the same when a cover is replaced, so caching is by version
/// (`?v=`, see `artwork::revision`): a URL naming the file being served is
/// cached for good -- a new cover is a new URL -- and anything else, an old
/// version or none, is revalidated every time, so it can never pin old art.
async fn art(State(s): State<Arc<Shared>>, Path(id): Path<String>, RawQuery(query): RawQuery) -> Response {
    // The id is a hex hash; reject anything else rather than touching the path.
    if !bard_core::artwork::is_valid_id(&id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let file = bard_core::artwork::cache_path(&s.artwork_dir, &id);
    let asked = query.as_deref().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("v=")).map(str::to_owned));
    // Before the read: a file replaced during it is then a mismatch, not
    // new bytes cached under the old name.
    let current = bard_core::artwork::revision(&s.artwork_dir, &id);
    let cache = if asked.is_some() && asked == current { "public, max-age=31536000, immutable" } else { "no-cache" };
    match tokio::fs::read(&file).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response(),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[overlay] art send failed: {e}");
            }
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// Counts a client for as long as its stream lives.
struct ClientGuard(Arc<Shared>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::Relaxed);
    }
}

/// SSE rather than polling: the overlay is told about a track change the
/// moment it happens instead of up to a second later.
async fn events(State(s): State<Arc<Shared>>) -> Response {
    let rx = s.frames.subscribe();
    let first: Arc<str> = frame(&s.latest.lock().unwrap()).into();
    s.clients.fetch_add(1, Ordering::Relaxed);
    let guard = ClientGuard(s.clone());

    let stream = futures_util::stream::unfold((Some(first), rx, guard), |(first, mut rx, guard)| async move {
        if let Some(f) = first {
            return Some((Ok::<_, std::convert::Infallible>(f.to_string()), (None, rx, guard)));
        }
        loop {
            match rx.recv().await {
                Ok(f) => return Some((Ok(f.to_string()), (None, rx, guard))),
                // A slow client missed some frames; the next one is a
                // complete snapshot, so just carry on.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-transform"));
    h.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_upward_from_a_taken_port_on_loopback_only() {
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let next = bind_with_retry(port).expect("a free port above");
        let addr = next.local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        assert!(addr.port() > port && addr.port() < port + PORT_ATTEMPTS);
    }

    #[test]
    fn frames_match_json_stringify() {
        let np = NowPlaying {
            title: "T".into(),
            artist: "A".into(),
            album: "Al".into(),
            duration_sec: 200.0,
            position_sec: 1.5,
            is_playing: true,
            art_url: None,
            queue_length: 3,
            updated_at: "2026-11-12T03:12:50.260Z".into(),
        };
        assert_eq!(
            frame(&np),
            "data: {\"title\":\"T\",\"artist\":\"A\",\"album\":\"Al\",\"durationSec\":200,\"positionSec\":1.5,\"isPlaying\":true,\"artUrl\":null,\"queueLength\":3,\"updatedAt\":\"2026-11-12T03:12:50.260Z\"}\n\n"
        );
    }

    #[test]
    fn embeds_the_overlay_page() {
        for f in ["index.html", "overlay.css", "overlay.js"] {
            assert!(OVERLAY.get_file(f).is_some(), "{f}");
        }
    }
}
