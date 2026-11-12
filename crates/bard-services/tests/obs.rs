//! The OBS watcher against a fake obs-websocket v5 server on loopback.

use bard_core::Bard;
use bard_core::types::StreamerSource;
use bard_services::obs::{self, ObsHandler, ObsWatch};
use bard_services::{AppControls, Services, ServicesOptions, secrets::ObsPassword};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

#[derive(Default)]
struct Rec {
    live: Mutex<Vec<StreamerSource>>,
    status: Mutex<Vec<String>>,
    password: String,
}
impl ObsHandler for Rec {
    fn live(&self, s: StreamerSource) {
        self.live.lock().unwrap().push(s);
    }
    fn status(&self, m: String) {
        self.status.lock().unwrap().push(m);
    }
    fn password(&self) -> impl Future<Output = String> + Send {
        let p = self.password.clone();
        async move { p }
    }
}

/// One fake OBS connection. `password`: require auth. `active`: which status
/// requests answer outputActive = true. `event`: an event to send after.
async fn fake_obs(listener: TcpListener, password: Option<&'static str>, active: &'static [&'static str], event: Option<Value>) {
    let (tcp, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
    let (salt, challenge) = ("c2FsdA==", "Y2hhbGxlbmdl");
    let mut hello = json!({"op":0,"d":{"obsWebSocketVersion":"5.5.0","rpcVersion":1}});
    if password.is_some() {
        hello["d"]["authentication"] = json!({"salt": salt, "challenge": challenge});
    }
    ws.send(Message::text(hello.to_string())).await.unwrap();
    let Some(Ok(Message::Text(identify))) = ws.next().await else { return };
    let identify: Value = serde_json::from_str(&identify).unwrap();
    assert_eq!(identify["op"], 1);
    assert_eq!(identify["d"]["eventSubscriptions"], 64);
    if let Some(pw) = password {
        if identify["d"]["authentication"] != obs::auth_string(pw, salt, challenge) {
            let frame = CloseFrame { code: 4009.into(), reason: "Authentication failed.".into() };
            ws.send(Message::Close(Some(frame))).await.unwrap();
            // Let the client see the close before the socket drops.
            while ws.next().await.is_some() {}
            return;
        }
    } else {
        assert!(identify["d"].get("authentication").is_none());
    }
    ws.send(Message::text(json!({"op":2,"d":{"negotiatedRpcVersion":1}}).to_string())).await.unwrap();
    for _ in 0..4 {
        let Some(Ok(Message::Text(req))) = ws.next().await else { return };
        let req: Value = serde_json::from_str(&req).unwrap();
        assert_eq!(req["op"], 6);
        let id = req["d"]["requestId"].as_str().unwrap().to_owned();
        let reply = json!({"op":7,"d":{"requestType":id,"requestId":id,
            "requestStatus":{"result":true,"code":100},
            "responseData":{"outputActive": active.contains(&id.as_str())}}});
        ws.send(Message::text(reply.to_string())).await.unwrap();
    }
    if let Some(e) = event {
        ws.send(Message::text(e.to_string())).await.unwrap();
    }
    // Then OBS "quits": the watcher must not disarm, only reconnect later.
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn arms_from_status_replies_and_events_with_auth() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let event = json!({"op":5,"d":{"eventType":"StreamStateChanged","eventIntent":64,
        "eventData":{"outputActive":true,"outputState":"OBS_WEBSOCKET_OUTPUT_STARTED"}}});
    let server = tokio::spawn(fake_obs(listener, Some("hunter2"), &["GetRecordStatus"], Some(event)));
    let rec = Arc::new(Rec { password: "hunter2".into(), ..Default::default() });
    let _watch = ObsWatch::start(&tokio::runtime::Handle::current(), url, rec.clone());
    wait_for(|| rec.live.lock().unwrap().len() >= 2).await;
    assert_eq!(*rec.live.lock().unwrap(), [StreamerSource::Record, StreamerSource::Stream]);
    assert!(rec.status.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn a_rejected_password_stops_and_says_so() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(fake_obs(listener, Some("right"), &[], None));
    let rec = Arc::new(Rec { password: "wrong".into(), ..Default::default() });
    let watch = ObsWatch::start(&tokio::runtime::Handle::current(), url, rec.clone());
    wait_for(|| watch.is_finished()).await;
    assert!(watch.is_finished(), "a rejected password is not retried");
    assert_eq!(rec.status.lock().unwrap().len(), 1);
    assert!(rec.status.lock().unwrap()[0].contains("rejected the password"));
    server.await.unwrap();
}

#[tokio::test]
async fn a_missing_password_stops_and_says_so() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(fake_obs(listener, Some("right"), &[], None));
    let rec = Arc::new(Rec::default());
    let watch = ObsWatch::start(&tokio::runtime::Handle::current(), url, rec.clone());
    wait_for(|| watch.is_finished()).await;
    assert!(watch.is_finished());
    assert!(rec.status.lock().unwrap()[0].contains("needs a password"));
    drop(server);
}

#[tokio::test]
async fn unreachable_obs_is_reported_once_and_retried() {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let rec = Arc::new(Rec::default());
    let watch = ObsWatch::start(&tokio::runtime::Handle::current(), format!("ws://127.0.0.1:{port}"), rec.clone());
    // First attempt fails, second comes after the 2s backoff.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!watch.is_finished());
    assert_eq!(rec.status.lock().unwrap().len(), 1, "{:?}", rec.status.lock().unwrap());
}

struct NoControls;
impl AppControls for NoControls {
    fn play(&self) {}
    fn pause(&self) {}
    fn toggle(&self) {}
    fn next(&self) {}
    fn previous(&self) {}
    fn seek(&self, _: f64) {}
    fn set_volume(&self, _: f64) {}
    fn show_window(&self, _: Option<String>) {}
    fn show_popup(&self) {}
    fn quit(&self) {}
}

/// End to end through Services: autoArm on, OBS already recording, and
/// streamer mode ends up armed in the core with the source named.
#[test]
fn services_arm_bard_from_obs() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = rt.spawn(fake_obs(listener, None, &["GetRecordStatus"], None));

    let dir = tempfile::tempdir().unwrap();
    let (mut bard, events) = Bard::open(dir.path().to_path_buf()).unwrap();
    bard.update_config(|c| {
        c.streamer.auto_arm = true;
        c.streamer.obs_url = url.clone();
    });
    let bard = Arc::new(Mutex::new(bard));
    let opts = ServicesOptions {
        overlay: false,
        mpris: false,
        tray: false,
        migrate_obs_password: false,
        obs_password: ObsPassword::with_service("bard-test-obs"),
        ..Default::default()
    };
    let services = Services::start(bard.clone(), Arc::new(NoControls), opts);
    for _ in 0..100 {
        if bard.lock().unwrap().streamer_source().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let b = bard.lock().unwrap();
    assert!(b.config().streamer.enabled, "armed");
    assert_eq!(b.streamer_source(), Some(StreamerSource::Record));
    drop(b);
    let notices: Vec<String> = events
        .try_iter()
        .filter_map(|e| match e {
            bard_core::CoreEvent::Notice(n) => Some(n),
            _ => None,
        })
        .collect();
    assert!(notices.iter().any(|n| n.contains("OBS went live")), "{notices:?}");
    services.shutdown();
    // OBS going away did not disarm.
    assert!(bard.lock().unwrap().config().streamer.enabled);
    rt.block_on(server).unwrap();
}
