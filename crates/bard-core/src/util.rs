//! Small helpers shared by the stores: JS-compatible JSON output, atomic
//! writes, and the wall clock.

use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Epoch milliseconds, as `Date.now()`.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `new Date(ms).toISOString()`: `2026-11-12T03:14:50.280Z`.
pub fn iso8601(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z", sod / 3600, (sod / 60) % 60, sod % 60)
}

/// How a store's file is indented, matching what the Electron build wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    /// `JSON.stringify(x)` -- stats.json.
    Compact,
    /// `JSON.stringify(x, null, 2)` -- albums.json, loved.json, nowplaying.json.
    Two,
    /// `JSON.stringify(x, undefined, '\t')` -- electron-store's config.json.
    Tab,
}

/// Serialises the way `JSON.stringify` would.
///
/// serde writes an integral f64 as `1.0`; JavaScript writes `1`. Both parse
/// the same, but a config file that flips `"volume": 1` to `1.0` on its first
/// Rust write is a needless diff for anyone keeping their dotfiles in git, so
/// whole floats are written as integers.
pub fn to_js_json<T: Serialize>(value: &T, indent: Indent) -> serde_json::Result<String> {
    let mut v = serde_json::to_value(value)?;
    integralise(&mut v);
    match indent {
        Indent::Compact => serde_json::to_string(&v),
        Indent::Two => serde_json::to_string_pretty(&v),
        Indent::Tab => {
            let mut out = Vec::new();
            let fmt = serde_json::ser::PrettyFormatter::with_indent(b"\t");
            let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
            v.serialize(&mut ser)?;
            Ok(String::from_utf8(out).expect("serde_json writes UTF-8"))
        }
    }
}

fn integralise(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|_| n.is_f64())
                && f.is_finite()
                && f.fract() == 0.0
                && f.abs() < 9.007_199_254_740_992e15
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(integralise),
        Value::Object(o) => o.values_mut().for_each(integralise),
        _ => {}
    }
}

/// Temp file then rename, so a reader never sees a half-written file and a
/// crash mid-write leaves the previous version intact.
pub fn write_atomic(target: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_javascript() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        // From a real nowplaying.json.
        assert_eq!(iso8601(1_794_454_490_280), "2026-11-12T03:34:50.280Z");
        assert_eq!(iso8601(951_782_400_123), "2000-02-29T00:00:00.123Z");
    }

    #[test]
    fn whole_floats_are_written_as_integers() {
        let v = serde_json::json!({ "a": 1.0, "b": 0.8, "c": [2.0, 3] });
        assert_eq!(to_js_json(&v, Indent::Compact).unwrap(), r#"{"a":1,"b":0.8,"c":[2,3]}"#);
    }

    #[test]
    fn tab_indent_matches_electron_store() {
        let v = serde_json::json!({ "a": [1] });
        assert_eq!(to_js_json(&v, Indent::Tab).unwrap(), "{\n\t\"a\": [\n\t\t1\n\t]\n}");
    }
}
