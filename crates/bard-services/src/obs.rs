//! obs-websocket v5 client, used for one thing: knowing when OBS goes live.
//! A port of `src/main/obs.ts`; the policy half (`streamer.ts`) is
//! [`ObsSync`] plus [`bard_core::Bard::arm`].
//!
//! The server ships inside OBS 28+ on port 4455 but is off by default, so this
//! is entirely opt-in and every failure path has to be survivable -- a user who
//! never enables it should notice nothing at all.
//!
//! Only ever *arms* streamer mode. Disarming is always deliberate, so a
//! websocket that drops mid-stream cannot silently remove protection, and a
//! reconnect cannot cut audio. That asymmetry is the whole safety argument: the
//! failure modes of this file are "did not arm when it could have", never "went
//! unprotected without saying so".
//!
//! Set `BARD_OBS_DEBUG=1` to log the handshake to stderr (never the password).

use bard_core::types::StreamerSource;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// Reconnect backoff. Caps so a closed OBS is cheap to wait on.
pub const RETRY: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// How long the websocket handshake may take before it counts as a failure.
/// Applied to each step of it: the upgrade, then OBS's Hello, then its
/// Identified. An endpoint that accepts the upgrade and then says nothing
/// (a hung OBS, a proxy, the wrong service on the port) otherwise held the
/// watcher forever with no notice and no retry. Once identified there is no
/// deadline: a quiet OBS that is not streaming sends nothing for hours.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);

/// obs-websocket close code for a rejected password.
pub const AUTH_FAILED: u16 = 4009;

/// Outputs that count as "going out".
///
/// Stream and record are the obvious two. Replay buffer and virtual camera are
/// included because they are equally capable of capturing audio that ends up
/// somewhere public -- a buffer clip saved to a highlight, a virtual camera fed
/// into a call being recorded at the other end.
///
/// The cost is that anyone leaving the replay buffer running permanently is
/// effectively always armed. That is the safe direction to be wrong in, and it
/// is still one click to turn off.
pub fn live_event(event_type: &str) -> Option<StreamerSource> {
    match event_type {
        "StreamStateChanged" => Some(StreamerSource::Stream),
        "RecordStateChanged" => Some(StreamerSource::Record),
        "ReplayBufferStateChanged" => Some(StreamerSource::Replay),
        "VirtualcamStateChanged" => Some(StreamerSource::Virtualcam),
        _ => None,
    }
}

/// Status requests asked on connect, so an already-live OBS arms immediately.
pub const LIVE_REQUESTS: [(&str, StreamerSource); 4] = [
    ("GetStreamStatus", StreamerSource::Stream),
    ("GetRecordStatus", StreamerSource::Record),
    ("GetReplayBufferStatus", StreamerSource::Replay),
    ("GetVirtualCamStatus", StreamerSource::Virtualcam),
];

fn live_request(request_id: &str) -> Option<StreamerSource> {
    LIVE_REQUESTS.iter().find(|(r, _)| *r == request_id).map(|(_, s)| *s)
}

/// obs-websocket v5 auth: base64(sha256(base64(sha256(password + salt)) + challenge)).
///
/// Only sent when the server offers a challenge; an unauthenticated server
/// omits it, and sending one anyway is rejected.
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(Sha256::digest(format!("{password}{salt}")));
    b64.encode(Sha256::digest(format!("{secret}{challenge}")))
}

/// What to do about one message from OBS. Pure, so the protocol is testable
/// without a server.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Hello: send Identify. `auth` is the (salt, challenge) when the server
    /// wants a password.
    Identify {
        auth: Option<(String, String)>,
    },
    /// Identified: ask whether we are already live.
    Identified,
    /// An output went live (event) or already was (status reply).
    Live(StreamerSource, String),
    Ignore,
}

pub fn step(raw: &str) -> Step {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else { return Step::Ignore };
    let d = &msg["d"];
    match msg["op"].as_u64() {
        // Hello -> Identify.
        Some(0) => {
            let auth = d
                .get("authentication")
                .and_then(|a| Some((a["salt"].as_str()?.to_owned(), a["challenge"].as_str()?.to_owned())));
            Step::Identify { auth }
        }
        Some(2) => Step::Identified,
        // Event.
        Some(5) => {
            let ty = d["eventType"].as_str().unwrap_or_default();
            match live_event(ty) {
                Some(src) if d["eventData"]["outputActive"] == Value::Bool(true) => Step::Live(src, ty.to_owned()),
                _ => Step::Ignore,
            }
        }
        // Request response. Each status request answers under its own field
        // names as well as outputActive; outputActive is the common one.
        Some(7) => {
            let id = d["requestId"].as_str().unwrap_or_default();
            match live_request(id) {
                Some(src) if d["responseData"]["outputActive"] == Value::Bool(true) => Step::Live(src, id.to_owned()),
                _ => Step::Ignore,
            }
        }
        _ => Step::Ignore,
    }
}

/// The Identify message. Subscribing to Outputs (1 << 6) is what delivers
/// StreamStateChanged and RecordStateChanged.
pub fn identify(authentication: Option<String>) -> String {
    let mut d = json!({ "rpcVersion": 1, "eventSubscriptions": 1 << 6 });
    if let Some(a) = authentication {
        d["authentication"] = Value::String(a);
    }
    json!({ "op": 1, "d": d }).to_string()
}

fn debug(msg: impl FnOnce() -> String) {
    if std::env::var_os("BARD_OBS_DEBUG").is_some() {
        eprintln!("[obs] {}", msg());
    }
}

/// Where the watcher reports to.
pub trait ObsHandler: Send + Sync + 'static {
    /// An output is live: arm (one direction only; see the module docs).
    fn live(&self, source: StreamerSource);
    /// Something the user should be told.
    fn status(&self, message: String);
    /// The stored password, or "" for none.
    fn password(&self) -> impl Future<Output = String> + Send;
}

/// A running watcher. Dropping it stops it.
pub struct ObsWatch {
    task: JoinHandle<()>,
}

impl ObsWatch {
    /// Starts watching `url`, reconnecting with backoff until stopped.
    pub fn start<H: ObsHandler>(rt: &Handle, url: String, handler: Arc<H>) -> ObsWatch {
        ObsWatch { task: rt.spawn(run(url, handler)) }
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for ObsWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How a connection ended.
enum End {
    /// Reconnect after the backoff.
    Retry,
    /// Stop for good (until the settings change): a rejected or missing
    /// password is not a transient fault -- retrying forever would hammer OBS
    /// and never say why.
    Stop,
}

async fn run<H: ObsHandler>(url: String, h: Arc<H>) {
    let mut retry = 0usize;
    // Set once so a flapping or closed OBS cannot spam the notice strip; reset
    // by a successful connection.
    let mut told_user = false;
    loop {
        debug(|| format!("connecting to {url}"));
        match connection(&url, &*h, &mut retry, &mut told_user).await {
            End::Stop => return,
            End::Retry => {}
        }
        let wait = RETRY[retry.min(RETRY.len() - 1)];
        retry += 1;
        tokio::time::sleep(wait).await;
    }
}

async fn connection<H: ObsHandler>(url: &str, h: &H, retry: &mut usize, told_user: &mut bool) -> End {
    let report_unreachable = |told: &mut bool| {
        // Errors here are ordinary: OBS closed, websocket server disabled,
        // wrong port. Reported once, never repeatedly.
        if !*told {
            *told = true;
            h.status(format!("Could not reach OBS at {url} \u{2014} streamer mode will not arm itself"));
        }
    };

    let ws = match tokio::time::timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::connect_async(url)).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => {
            debug(|| format!("connect failed: {e}"));
            report_unreachable(told_user);
            return End::Retry;
        }
        Err(_) => {
            debug(|| "handshake timed out".into());
            report_unreachable(told_user);
            return End::Retry;
        }
    };
    let (mut tx, mut rx) = ws.split();

    // Until Identified, each message OBS owes has a deadline.
    let mut deadline = Some(tokio::time::Instant::now() + HANDSHAKE_TIMEOUT);
    loop {
        let next = match deadline {
            Some(d) => match tokio::time::timeout_at(d, rx.next()).await {
                Ok(m) => m,
                Err(_) => {
                    debug(|| "OBS did not finish identifying in time".into());
                    if !*told_user {
                        *told_user = true;
                        h.status(format!(
                            "OBS at {url} did not finish connecting \u{2014} streamer mode will not arm itself"
                        ));
                    }
                    return End::Retry;
                }
            },
            None => rx.next().await,
        };
        let Some(msg) = next else { break };
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(frame)) => {
                let code = frame.as_ref().map(|f| u16::from(f.code));
                debug(|| format!("closed {code:?}"));
                if code == Some(AUTH_FAILED) {
                    h.status("OBS rejected the password for its WebSocket server \u{2014} check it in Settings".into());
                    return End::Stop;
                }
                return End::Retry;
            }
            Ok(_) => continue,
            Err(e) => {
                debug(|| format!("socket error: {e}"));
                report_unreachable(told_user);
                return End::Retry;
            }
        };
        match step(&text) {
            Step::Identify { auth } => {
                let authentication = match auth {
                    Some((salt, challenge)) => {
                        let password = h.password().await;
                        if password.is_empty() {
                            h.status(
                                "OBS needs a password for its WebSocket server \u{2014} set one in Settings".into(),
                            );
                            return End::Stop;
                        }
                        // Length only, never the password itself: enough to
                        // tell "decrypted to nothing useful" from "wrong
                        // password" without putting a credential in a log.
                        debug(|| format!("password read, length {}", password.chars().count()));
                        Some(auth_string(&password, &salt, &challenge))
                    }
                    None => None,
                };
                debug(|| format!("sending Identify; auth challenge present: {}", authentication.is_some()));
                if tx.send(Message::text(identify(authentication))).await.is_err() {
                    return End::Retry;
                }
                // Reading the password may have waited on a keyring; OBS's
                // reply gets its own full allowance.
                deadline = Some(tokio::time::Instant::now() + HANDSHAKE_TIMEOUT);
            }
            Step::Identified => {
                deadline = None;
                *retry = 0;
                *told_user = false;
                debug(|| "identified; asking current output state".into());
                for (request_type, _) in LIVE_REQUESTS {
                    let req = json!({ "op": 6, "d": { "requestType": request_type, "requestId": request_type } });
                    if tx.send(Message::text(req.to_string())).await.is_err() {
                        return End::Retry;
                    }
                }
            }
            Step::Live(source, label) => {
                debug(|| format!("arming from {label}"));
                h.live(source);
            }
            Step::Ignore => {}
        }
    }
    End::Retry
}

/// Brings the watcher in line with config (`syncObsWatch`): running against
/// `streamer.obsUrl` while `streamer.autoArm` is on, stopped otherwise.
///
/// Restarts only when one of those two changed, or when forced (a new
/// password), so the many unrelated config changes do not drop and remake the
/// connection.
#[derive(Default)]
pub struct ObsSync {
    applied: Option<(bool, String)>,
    watch: Option<ObsWatch>,
}

impl ObsSync {
    pub fn sync<H: ObsHandler>(&mut self, rt: &Handle, auto_arm: bool, url: &str, force: bool, h: &Arc<H>) {
        let want = (auto_arm, url.to_owned());
        if !force && self.applied.as_ref() == Some(&want) {
            return;
        }
        self.applied = Some(want);
        self.watch = None;
        if auto_arm {
            self.watch = Some(ObsWatch::start(rt, url.to_owned(), h.clone()));
        }
    }

    pub fn stop(&mut self) {
        self.watch = None;
        self.applied = None;
    }

    pub fn is_running(&self) -> bool {
        self.watch.as_ref().is_some_and(|w| !w.is_finished())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_matches_the_obs_websocket_spec_example() {
        // Worked by hand from the protocol doc's algorithm with Node's crypto
        // (scripts/check-obs-auth.mjs's authString):
        //   node -e "const c=require('crypto');const s=c.createHash('sha256').update('supersecretpassword'+'lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=').digest('base64');console.log(c.createHash('sha256').update(s+'+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=').digest('base64'))"
        assert_eq!(
            auth_string(
                "supersecretpassword",
                "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
                "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="
            ),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }

    #[test]
    fn hello_with_and_without_auth() {
        let hello = r#"{"op":0,"d":{"obsWebSocketVersion":"5.0.0","rpcVersion":1,"authentication":{"challenge":"c","salt":"s"}}}"#;
        assert_eq!(step(hello), Step::Identify { auth: Some(("s".into(), "c".into())) });
        assert_eq!(step(r#"{"op":0,"d":{"rpcVersion":1}}"#), Step::Identify { auth: None });
    }

    #[test]
    fn identify_subscribes_to_outputs() {
        let v: Value = serde_json::from_str(&identify(None)).unwrap();
        assert_eq!(v, json!({"op":1,"d":{"rpcVersion":1,"eventSubscriptions":64}}));
        let v: Value = serde_json::from_str(&identify(Some("x".into()))).unwrap();
        assert_eq!(v["d"]["authentication"], "x");
    }

    #[test]
    fn only_outputs_going_active_arm() {
        let ev = |ty: &str, active: bool| {
            json!({"op":5,"d":{"eventType":ty,"eventData":{"outputActive":active,"outputState":"x"}}}).to_string()
        };
        assert_eq!(
            step(&ev("StreamStateChanged", true)),
            Step::Live(StreamerSource::Stream, "StreamStateChanged".into())
        );
        assert_eq!(
            step(&ev("VirtualcamStateChanged", true)),
            Step::Live(StreamerSource::Virtualcam, "VirtualcamStateChanged".into())
        );
        // Stopping never disarms: it is simply ignored.
        assert_eq!(step(&ev("RecordStateChanged", false)), Step::Ignore);
        assert_eq!(step(&ev("SceneChanged", true)), Step::Ignore);
    }

    #[test]
    fn status_replies_arm_when_already_live() {
        let reply = |id: &str, active: bool| {
            json!({"op":7,"d":{"requestType":id,"requestId":id,"requestStatus":{"result":true,"code":100},"responseData":{"outputActive":active}}}).to_string()
        };
        assert_eq!(
            step(&reply("GetReplayBufferStatus", true)),
            Step::Live(StreamerSource::Replay, "GetReplayBufferStatus".into())
        );
        assert_eq!(step(&reply("GetStreamStatus", false)), Step::Ignore);
        assert_eq!(step(&reply("GetSceneList", true)), Step::Ignore);
        assert_eq!(step("not json"), Step::Ignore);
        assert_eq!(step(r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#), Step::Identified);
    }
}
