//! Album order, random picks and shuffling. A port of `src/shared/playlist.ts`.

use crate::types::Track;
use std::cmp::Ordering;
use std::collections::HashSet;

/// A source of uniform floats in `[0, 1)`, like `Math.random`.
///
/// Injectable so shuffling and picking are deterministic under test.
pub type Rng<'a> = &'a mut dyn FnMut() -> f64;

/// `Math.random`.
pub fn math_random() -> f64 {
    fastrand::f64()
}

/// `Math.floor(rng() * n)`, clamped so an rng that returns exactly 1.0 cannot
/// index past the end.
pub(crate) fn pick_index(rng: Rng, n: usize) -> usize {
    ((rng() * n as f64).floor() as usize).min(n.saturating_sub(1))
}

/// Album order: paths carry the track number, so path order is release order.
///
/// The Electron build used `localeCompare(b, undefined, { numeric: true,
/// sensitivity: 'base' })`; see [`locale_compare`] for how that is reproduced.
pub fn by_path_order(a: &Track, b: &Track) -> Ordering {
    locale_compare(&a.path, &b.path)
}

/// An approximation of ICU root collation with `numeric: true,
/// sensitivity: 'base'`, which is what `String.localeCompare` does in
/// Chromium and Node.
///
/// What matters for album order, and what is reproduced exactly:
/// - runs of digits compare by value (`2` before `10`; leading zeros ignored),
/// - case and accents are ignored (`base` sensitivity),
/// - whitespace and punctuation sort before digits, digits before letters,
///   with ASCII punctuation in the root collation's order.
///
/// Letters outside Latin compare by their base code point, which is not what
/// ICU does for every script, but is stable and good enough for track order.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let ka = collation_elements(a);
    let kb = collation_elements(b);
    ka.cmp(&kb)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Element {
    /// (class, weight). Classes: 0 whitespace, 1 punctuation/symbols.
    Variable(u8, u32),
    /// A digit run: (significant digit count, digits) -- compares by value.
    Number(usize, Vec<u8>),
    /// A letter (or anything else), lowercased and stripped of accents.
    Letter(u32),
}

/// Root-collation order of ASCII punctuation and symbols (CLDR / DUCET).
const PUNCT_ORDER: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn collation_elements(s: &str) -> Vec<Element> {
    use unicode_normalization::UnicodeNormalization;
    use unicode_normalization::char::is_combining_mark;

    let mut out = Vec::new();
    let chars: Vec<char> = s.nfd().filter(|c| !is_combining_mark(*c)).collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digits: Vec<u8> = chars[start..i]
                .iter()
                .map(|c| *c as u8)
                .skip_while(|d| *d == b'0')
                .collect();
            out.push(Element::Number(digits.len(), digits));
            continue;
        }
        i += 1;
        if c.is_control() {
            continue; // ignorable
        }
        if c.is_whitespace() {
            out.push(Element::Variable(0, c as u32));
        } else if let Some(pos) = PUNCT_ORDER.find(c) {
            out.push(Element::Variable(1, pos as u32));
        } else if !c.is_alphanumeric() {
            // Typographic punctuation and other symbols: after the ASCII set.
            let w = match c {
                '\u{2018}' | '\u{2019}' => PUNCT_ORDER.find('\'').unwrap() as u32,
                '\u{201C}' | '\u{201D}' => PUNCT_ORDER.find('"').unwrap() as u32,
                '\u{2013}' | '\u{2014}' => PUNCT_ORDER.find('-').unwrap() as u32,
                _ => 0x100 + c as u32,
            };
            out.push(Element::Variable(1, w));
        } else {
            for l in c.to_lowercase() {
                out.push(Element::Letter(l as u32));
            }
        }
    }
    out
}

/// `a < b` the way JavaScript's default sort compares strings: by UTF-16 code
/// units, not by code points.
pub fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

pub fn tracks_in_album(tracks: &[Track], album: &str) -> Vec<Track> {
    let mut out: Vec<Track> = tracks.iter().filter(|t| t.album == album).cloned().collect();
    out.sort_by(by_path_order);
    out
}

pub fn album_names(tracks: &[Track]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names: Vec<String> = tracks
        .iter()
        .filter(|t| seen.insert(t.album.as_str()))
        .map(|t| t.album.clone())
        .collect();
    names.sort_by(|a, b| utf16_cmp(a, b));
    names
}

/// A random track, avoiding `exclude`.
///
/// Pure random repeats itself uncomfortably often -- on a few hundred tracks
/// there is a better-than-even chance of a repeat within ~30 picks. Excluding a
/// recent window is what makes shuffle feel shuffled. If the window has grown to
/// cover everything, it is ignored rather than returning nothing.
pub fn pick_random<'t>(
    tracks: &'t [Track],
    exclude: &HashSet<String>,
    rng: Rng,
) -> Option<&'t Track> {
    if tracks.is_empty() {
        return None;
    }
    let pool: Vec<&Track> = tracks.iter().filter(|t| !exclude.contains(&t.id)).collect();
    let from: Vec<&Track> = if pool.is_empty() { tracks.iter().collect() } else { pool };
    Some(from[pick_index(rng, from.len())])
}

/// The next track on the same album, or None at the end of it.
pub fn next_in_album(tracks: &[Track], track: &Track) -> Option<Track> {
    let album = tracks_in_album(tracks, &track.album);
    let i = album.iter().position(|t| t.id == track.id)?;
    album.into_iter().nth(i + 1)
}

/// First track of a random album other than `not_this`.
pub fn random_album_start(tracks: &[Track], not_this: Option<&str>, rng: Rng) -> Option<Track> {
    let names: Vec<String> =
        album_names(tracks).into_iter().filter(|a| Some(a.as_str()) != not_this).collect();
    let pick = if names.is_empty() { album_names(tracks) } else { names };
    if pick.is_empty() {
        return None;
    }
    let name = &pick[pick_index(rng, pick.len())];
    tracks_in_album(tracks, name).into_iter().next()
}

/// The next track after `anchor` in album order, rolling to another album when
/// the current one ends.
///
/// Takes the anchor as an argument rather than reading "what is playing": when
/// filling several tracks ahead, the anchor has to advance with each pick.
/// Anchoring on the current track instead returns the same successor every
/// time, which silently fills nothing.
pub fn next_in_album_rolling(tracks: &[Track], anchor: Option<&Track>, rng: Rng) -> Option<Track> {
    match anchor {
        None => random_album_start(tracks, None, rng),
        Some(a) => next_in_album(tracks, a)
            .or_else(|| random_album_start(tracks, Some(&a.album), rng)),
    }
}

/// Fisher-Yates. Returns a new vector; the input is untouched.
pub fn shuffled<T: Clone>(items: &[T], rng: Rng) -> Vec<T> {
    let mut next = items.to_vec();
    for i in (1..next.len()).rev() {
        let j = pick_index(rng, i + 1);
        next.swap(i, j);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(id: &str, album: &str, n: u32) -> Track {
        Track {
            id: id.into(),
            // Track order lives in the filename, which is why album order is path order.
            path: format!("/music/{album}/{n:02} - {id}.flac"),
            title: id.into(),
            artist: "Harris Heller".into(),
            album: album.into(),
            duration_sec: 100.0,
            has_artwork: true,
            search_text: id.into(),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        }
    }

    fn tracks() -> Vec<Track> {
        vec![
            make("gold3", "Gold", 3),
            make("gold1", "Gold", 1),
            make("gold2", "Gold", 2),
            make("rain1", "Rain", 1),
            make("rain2", "Rain", 2),
        ]
    }

    /// Deterministic rng returning each value in turn, then repeating the last.
    fn seq(values: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut i = 0;
        move || {
            let v = values[i.min(values.len() - 1)];
            i += 1;
            v
        }
    }

    fn ids(ts: &[Track]) -> Vec<&str> {
        ts.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn tracks_in_album_returns_release_order_regardless_of_input_order() {
        assert_eq!(ids(&tracks_in_album(&tracks(), "Gold")), ["gold1", "gold2", "gold3"]);
    }

    #[test]
    fn tracks_in_album_is_empty_for_an_unknown_album() {
        assert!(tracks_in_album(&tracks(), "Nope").is_empty());
    }

    #[test]
    fn album_names_lists_each_once_sorted() {
        assert_eq!(album_names(&tracks()), ["Gold", "Rain"]);
    }

    #[test]
    fn next_in_album_advances() {
        let first = tracks_in_album(&tracks(), "Gold")[0].clone();
        assert_eq!(next_in_album(&tracks(), &first).unwrap().id, "gold2");
    }

    #[test]
    fn next_in_album_is_none_at_the_end() {
        let last = tracks_in_album(&tracks(), "Gold")[2].clone();
        assert!(next_in_album(&tracks(), &last).is_none());
    }

    #[test]
    fn next_in_album_is_none_for_a_track_not_in_the_library() {
        assert!(next_in_album(&tracks(), &make("ghost", "Gold", 9)).is_none());
    }

    #[test]
    fn random_album_start_starts_at_the_first_track() {
        assert_eq!(random_album_start(&tracks(), None, &mut seq(&[0.0])).unwrap().id, "gold1");
    }

    #[test]
    fn random_album_start_avoids_the_album_given() {
        // Only Rain remains once Gold is excluded.
        let t = random_album_start(&tracks(), Some("Gold"), &mut seq(&[0.0])).unwrap();
        assert_eq!(t.id, "rain1");
    }

    #[test]
    fn random_album_start_falls_back_when_excluding_leaves_nothing() {
        let one: Vec<Track> = tracks().into_iter().filter(|t| t.album == "Gold").collect();
        assert_eq!(random_album_start(&one, Some("Gold"), &mut seq(&[0.0])).unwrap().id, "gold1");
    }

    #[test]
    fn random_album_start_is_none_for_an_empty_library() {
        assert!(random_album_start(&[], None, &mut seq(&[0.0])).is_none());
    }

    #[test]
    fn pick_random_never_returns_an_excluded_track() {
        let exclude: HashSet<String> =
            ["gold1", "gold2", "gold3", "rain1"].iter().map(|s| s.to_string()).collect();
        let ts = tracks();
        // Whatever the rng says, only rain2 is eligible.
        for r in [0.0, 0.4, 0.99] {
            assert_eq!(pick_random(&ts, &exclude, &mut || r).unwrap().id, "rain2");
        }
    }

    #[test]
    fn pick_random_ignores_an_exclusion_covering_everything() {
        // Better to repeat than to fall silent mid-stream.
        let ts = tracks();
        let all: HashSet<String> = ts.iter().map(|t| t.id.clone()).collect();
        assert!(pick_random(&ts, &all, &mut seq(&[0.0])).is_some());
    }

    #[test]
    fn pick_random_is_none_for_an_empty_library() {
        assert!(pick_random(&[], &HashSet::new(), &mut seq(&[0.0])).is_none());
    }

    #[test]
    fn shuffled_keeps_every_item_once() {
        let ts = tracks();
        let out = shuffled(&ts, &mut seq(&[0.9, 0.1, 0.7, 0.3]));
        assert_eq!(out.len(), ts.len());
        let a: HashSet<_> = out.iter().map(|t| &t.id).collect();
        let b: HashSet<_> = ts.iter().map(|t| &t.id).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn shuffled_does_not_mutate_the_input() {
        let input = tracks();
        let _ = shuffled(&input, &mut seq(&[0.5]));
        assert_eq!(ids(&input), ids(&tracks()));
    }

    #[test]
    fn shuffled_handles_empty_and_single() {
        assert!(shuffled::<&str>(&[], &mut seq(&[0.0])).is_empty());
        assert_eq!(shuffled(&["a"], &mut seq(&[0.0])), ["a"]);
    }

    #[test]
    fn shuffled_actually_reorders_for_a_given_seed() {
        let items = ["a", "b", "c", "d", "e"];
        assert_ne!(shuffled(&items, &mut seq(&[0.99, 0.01, 0.99, 0.01])), items);
    }

    #[test]
    fn locale_compare_is_numeric_and_case_and_accent_blind() {
        use Ordering::*;
        assert_eq!(locale_compare("2 - b", "10 - a"), Less);
        assert_eq!(locale_compare("01 - x", "1 - x"), Equal);
        assert_eq!(locale_compare("Abc", "abc"), Equal);
        assert_eq!(locale_compare("é", "e"), Equal);
        assert_eq!(locale_compare("a b", "ab"), Less); // space sorts before letters
        assert_eq!(locale_compare("a_b", "a-b"), Less);
        assert_eq!(locale_compare("Music/Z", "Music-secrets"), Greater); // '-' < '/'
        assert_eq!(locale_compare("track 9", "Track 10"), Less);
    }
}
