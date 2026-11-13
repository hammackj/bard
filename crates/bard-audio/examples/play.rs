//! Smoke test you can hear: plays files through the system's audio output.
//!
//!     cargo run -p bard-audio --example play -- --list
//!     cargo run -p bard-audio --example play -- [--output NODE] [--volume 0.5]
//!         [--seek SECS] [--crossfade SECS] FILE [FILE...]
//!
//! Files play in order; with --crossfade the next one starts that many
//! seconds before the current one ends, the way the app's queue drives it.
//! Shows position, live gain and the analyser's peak as it goes. Keys are not
//! read; Ctrl-C to stop.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use bard_audio::{FFT_SIZE, Player, PlayerEvent};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut files = Vec::new();
    let (mut output, mut volume, mut seek, mut crossfade, mut list) = (None, 0.5f32, None, 0.0f32, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--list" => list = true,
            "--output" => output = args.next(),
            "--volume" => volume = args.next().and_then(|v| v.parse().ok()).unwrap_or(volume),
            "--seek" => seek = args.next().and_then(|v| v.parse::<f64>().ok()),
            "--crossfade" => crossfade = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            _ => files.push(PathBuf::from(a)),
        }
    }

    let (player, events) = Player::new()?;
    if list || files.is_empty() {
        println!("sinks (pass the first column to --output):");
        for o in player.outputs() {
            println!("  {:<50} {}", o.id, o.name);
        }
        if files.is_empty() {
            return Ok(());
        }
    }
    if let Some(id) = &output
        && let Err(e) = player.set_output(id)
    {
        eprintln!("output: {e}");
    }
    player.set_volume(volume);

    let analyser = player.analyser();
    let mut wave = vec![0u8; FFT_SIZE];
    let mut next = 0;
    let start_next = |player: &Player, next: &mut usize, xfade: f32| -> bool {
        let Some(f) = files.get(*next) else { return false };
        println!("\n> {}", f.display());
        player.play(format!("track-{next}"), f.clone(), 1.0, xfade);
        *next += 1;
        true
    };
    start_next(&player, &mut next, 0.0);
    if let Some(s) = seek {
        player.seek(s);
    }

    let mut queued_early = false;
    loop {
        while let Ok(e) = events.try_recv() {
            match e {
                PlayerEvent::Ended { .. } => {
                    if !start_next(&player, &mut next, 0.0) {
                        println!("\ndone");
                        return Ok(());
                    }
                    queued_early = false;
                }
                PlayerEvent::Error(m) => eprintln!("\nerror: {m}"),
                PlayerEvent::Failed { message, .. } => {
                    eprintln!("\ncould not open: {message}");
                    if !start_next(&player, &mut next, 0.0) {
                        return Ok(());
                    }
                }
                PlayerEvent::Progress { track_id, .. } => {
                    // A crossfaded track has taken over.
                    if track_id.as_deref() == Some(&format!("track-{}", next.saturating_sub(1))) {
                        queued_early = false;
                    }
                }
            }
        }
        let p = player.position();
        if crossfade > 0.0
            && !queued_early
            && p.playing
            && p.duration > 0.0
            && p.duration - p.position <= crossfade as f64
            && start_next(&player, &mut next, crossfade)
        {
            queued_early = true;
        }
        analyser.time_domain(&mut wave);
        let peak = wave.iter().map(|b| b.abs_diff(128)).max().unwrap_or(0) as usize;
        let bar = "#".repeat(peak * 30 / 128);
        print!(
            "\r{:>7.2} / {:>7.2}  {}  gain {:.3}  {:<30}",
            p.position,
            p.duration,
            if p.playing { "playing" } else { "paused " },
            player.current_gain(),
            bar
        );
        std::io::stdout().flush().ok();
        std::thread::sleep(Duration::from_millis(33));
    }
}
