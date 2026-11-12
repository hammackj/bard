//! Track search: the library filter, and the fuzzy best-match from
//! `src/shared/match.ts`.
//!
//! The Electron build used fuse.js 7.5 for the fuzzy matcher. Its behaviour
//! with the options match.ts set (one key, `ignoreLocation`, threshold 0.4,
//! field-length norm on) is reproduced here exactly -- the Bitap search is a
//! line-for-line port operating on UTF-16 code units as JavaScript does -- so
//! scores and therefore accept/reject decisions match the TypeScript tests,
//! which were tuned against a real library.

use crate::types::Track;

/// Fuse scores run 0 (perfect) to 1 (nothing).
///
/// 0.4 is deliberately conservative. Measured against a real 585-track library
/// it tolerates single-character typos ("Ambent Gold", "Cofee Nook") and
/// rejects every out-of-library probe. At 0.5 it additionally handles
/// transpositions, but "lofi" starts matching an unrelated track -- exactly the
/// failure a request bot must not have, since a wrong match plays on stream
/// while a miss just replies "not found".
pub const MATCH_THRESHOLD: f64 = 0.4;

/// What search indexes. Kept here so scanning and matching cannot disagree.
pub fn search_text_for(title: &str, artist: &str, album: &str) -> String {
    format!("{title} {artist} {album}").to_lowercase()
}

/// The library pane's filter: a case-insensitive substring of `searchText`.
///
/// This is what the Electron renderer did (`searchText.includes(q)`), not a
/// fuzzy search -- a filter that admits near-misses makes a long list harder
/// to scan, not easier. An empty or blank query matches everything.
pub fn filter<'t>(tracks: &'t [Track], query: &str) -> Vec<&'t Track> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return tracks.iter().collect();
    }
    tracks.iter().filter(|t| t.search_text.contains(&q)).collect()
}

/// A fuzzy best-match over a track list.
///
/// Returns None rather than a weak best-effort hit: someone typing something
/// vague should get "not found", not whichever track scored least badly.
pub struct Matcher<'t> {
    records: Vec<(&'t Track, Vec<u16>, f64)>,
}

impl<'t> Matcher<'t> {
    pub fn new(tracks: &'t [Track]) -> Self {
        let records =
            tracks.iter().map(|t| (t, t.search_text.encode_utf16().collect(), field_norm(&t.search_text))).collect();
        Matcher { records }
    }

    pub fn best(&self, query: &str) -> Option<&'t Track> {
        self.best_scored(query).map(|(t, _)| t)
    }

    /// The best match and its Fuse score, or None when nothing is under the
    /// threshold.
    pub fn best_scored(&self, query: &str) -> Option<(&'t Track, f64)> {
        self.fuse_top(query).filter(|(_, s)| *s <= MATCH_THRESHOLD)
    }

    /// What `fuse.search(q, { limit: 1 })[0]` returns: the top result, which
    /// can still score above the threshold once the field-length norm is
    /// applied -- match.ts filtered those out afterwards, as `best_scored` does.
    fn fuse_top(&self, query: &str) -> Option<(&'t Track, f64)> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        let searcher = Bitap::new(q);
        let mut best: Option<(&'t Track, f64)> = None;
        for (track, text, norm) in &self.records {
            let Some(score) = searcher.search_in(text) else { continue };
            // computeScoreSingle: one key of weight 1.
            let base = if score == 0.0 { f64::EPSILON } else { score };
            let total = base.powf(*norm);
            // Ties go to the earlier record, as Fuse's sort does.
            if best.is_none_or(|(_, s)| total < s) {
                best = Some((track, total));
            }
        }
        best
    }
}

/// fuse.js `norm(1, 3)`: 1/sqrt(word count), rounded to three places.
fn field_norm(value: &str) -> f64 {
    let is_sep = |c: u16| (9..=13).contains(&c) || c == 32 || c == 160;
    let mut tokens = 0;
    let mut in_word = false;
    for c in value.encode_utf16() {
        if !is_sep(c) {
            if !in_word {
                tokens += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    let tokens = tokens.max(1);
    (1000.0 / (tokens as f64).sqrt()).round() / 1000.0
}

const MAX_BITS: usize = 32;

struct Chunk {
    pattern: Vec<u16>,
    alphabet: std::collections::HashMap<u16, u32>,
}

/// fuse.js `BitapSearch` with the options match.ts uses.
struct Bitap {
    pattern: Vec<u16>,
    chunks: Vec<Chunk>,
}

impl Bitap {
    fn new(pattern: &str) -> Self {
        let pattern: Vec<u16> = pattern.to_lowercase().encode_utf16().collect();
        let add = |p: &[u16]| {
            let mut alphabet = std::collections::HashMap::new();
            for (i, c) in p.iter().enumerate() {
                *alphabet.entry(*c).or_insert(0u32) |= 1u32 << (p.len() - i - 1);
            }
            Chunk { pattern: p.to_vec(), alphabet }
        };
        let len = pattern.len();
        let mut chunks = Vec::new();
        if len > MAX_BITS {
            let remainder = len % MAX_BITS;
            let end = len - remainder;
            let mut i = 0;
            while i < end {
                chunks.push(add(&pattern[i..i + MAX_BITS]));
                i += MAX_BITS;
            }
            if remainder > 0 {
                chunks.push(add(&pattern[len - MAX_BITS..]));
            }
        } else if len > 0 {
            chunks.push(add(&pattern));
        }
        Bitap { pattern, chunks }
    }

    /// The Bitap score, or None when the text does not match. `text` is
    /// already lowercase: searchText is built that way.
    fn search_in(&self, text: &[u16]) -> Option<f64> {
        if self.pattern == text {
            return Some(0.0);
        }
        let mut total = 0.0;
        let mut any = false;
        for chunk in &self.chunks {
            let (is_match, score) = bitap_search(text, &chunk.pattern, &chunk.alphabet);
            any |= is_match;
            total += score;
        }
        any.then(|| total / self.chunks.len() as f64)
    }
}

/// fuse.js `search()` with `ignoreLocation: true`, `location: 0`,
/// `findAllMatches: false`, `minMatchCharLength: 1`, no match indices.
///
/// Kept in the shape of the original rather than simplified to an edit
/// distance, so that its quirks (which score is reported, where it stops)
/// carry over unchanged. JavaScript's int32 bit operations are mirrored with
/// wrapping u32 arithmetic.
fn bitap_search(text: &[u16], pattern: &[u16], alphabet: &std::collections::HashMap<u16, u32>) -> (bool, f64) {
    let pattern_len = pattern.len();
    let text_len = text.len();
    let calc_score = |errors: usize| errors as f64 / pattern_len as f64;

    let mut current_threshold = MATCH_THRESHOLD;
    // Exact matches first, as a speed-up: any exact hit makes the threshold 0.
    if text.windows(pattern_len).any(|w| w == pattern) {
        current_threshold = current_threshold.min(calc_score(0));
    }

    let mut best_location: i64 = -1;
    let mut last_bit_arr: Vec<u32> = Vec::new();
    let mut final_score = 1.0;
    let mut bin_max = pattern_len + text_len;
    let mask = 1u32 << (pattern_len - 1);
    let get = |v: &Vec<u32>, i: usize| v.get(i).copied().unwrap_or(0);

    for i in 0..pattern_len {
        // With ignoreLocation the score does not depend on location, so the
        // binary search either keeps the whole window or collapses it.
        let mut bin_min = 0;
        let mut bin_mid = bin_max;
        while bin_min < bin_mid {
            if calc_score(i) <= current_threshold {
                bin_min = bin_mid;
            } else {
                bin_max = bin_mid;
            }
            bin_mid = (bin_max - bin_min) / 2 + bin_min;
        }
        bin_max = bin_mid;

        let start = 1usize;
        let finish = bin_mid.min(text_len) + pattern_len;
        let mut bit_arr = vec![0u32; finish + 2];
        bit_arr[finish + 1] = (1u32 << i).wrapping_sub(1);

        let mut j = finish;
        while j >= start {
            let current_location = j - 1;
            let char_match = text.get(current_location).and_then(|c| alphabet.get(c)).copied().unwrap_or(0);
            bit_arr[j] = ((bit_arr[j + 1] << 1) | 1) & char_match;
            if i > 0 {
                let prev = get(&last_bit_arr, j + 1) | get(&last_bit_arr, j);
                bit_arr[j] |= (prev << 1) | 1 | get(&last_bit_arr, j + 1);
            }
            if bit_arr[j] & mask != 0 {
                final_score = calc_score(i);
                if final_score <= current_threshold {
                    current_threshold = final_score;
                    best_location = current_location as i64;
                    // expectedLocation is 0, so any hit at 0 ends the row; the
                    // start bound otherwise stays at 1.
                    if best_location <= 0 {
                        break;
                    }
                }
            }
            j -= 1;
        }

        if calc_score(i + 1) > current_threshold {
            break;
        }
        last_bit_arr = bit_arr;
    }

    (best_location >= 0, final_score.max(0.001))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(id: &str, title: &str, artist: &str, album: &str) -> Track {
        Track {
            id: id.into(),
            path: format!("/music/{album}/{title}.flac"),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_sec: 120.0,
            has_artwork: true,
            search_text: search_text_for(title, artist, album),
            track_gain_db: None,
            album_gain_db: None,
            track_peak: None,
            album_peak: None,
        }
    }

    // Real titles from the StreamBeats lofi catalogue this was developed against.
    fn tracks() -> Vec<Track> {
        vec![
            make("a1", "Ambient Gold", "Harris Heller", "Gold"),
            make("a2", "No One\u{2019}s Hero", "Harris Heller", "Gold"),
            make("b1", "Twinkle Tunes", "Harris Heller", "Electric"),
            make("b2", "Neon in the Metropolis", "Harris Heller", "Electric"),
            make("c1", "Coffee Nook", "Harris Heller", "Prime"),
            make("c2", "Cherry Blossoms", "Harris Heller", "Moonwalk"),
            make("d1", "Butterfly Effect", "Harris Heller", "Rain"),
            make("d2", "Doin\u{2019} Fine", "Harris Heller", "Rain"),
        ]
    }

    fn title_of(q: &str) -> Option<String> {
        let ts = tracks();
        Matcher::new(&ts).best(q).map(|t| t.title.clone())
    }

    #[test]
    fn matches_an_exact_title() {
        assert_eq!(title_of("Ambient Gold").as_deref(), Some("Ambient Gold"));
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(title_of("ambient gold").as_deref(), Some("Ambient Gold"));
    }

    #[test]
    fn matches_a_partial_title() {
        assert_eq!(title_of("Twinkle").as_deref(), Some("Twinkle Tunes"));
    }

    #[test]
    fn matches_a_single_character_typo() {
        assert_eq!(title_of("Ambent Gold").as_deref(), Some("Ambient Gold"));
        assert_eq!(title_of("Cofee Nook").as_deref(), Some("Coffee Nook"));
    }

    #[test]
    fn matches_a_trailing_truncation() {
        assert_eq!(title_of("Butterfly Effec").as_deref(), Some("Butterfly Effect"));
    }

    #[test]
    fn matches_on_artist_name() {
        assert!(title_of("Harris Heller").is_some());
    }

    #[test]
    fn matches_on_album_name() {
        let ts = tracks();
        assert_eq!(Matcher::new(&ts).best("Metropolis").unwrap().album, "Electric");
    }

    #[test]
    fn refuses_weak_matches() {
        // A wrong match plays on stream; a miss just replies "not found".
        for q in ["zzzzqqqqxxxx", "Never Gonna Give You Up", "Bohemian Rhapsody", "shape of you"] {
            assert_eq!(title_of(q), None, "{q}");
        }
    }

    #[test]
    fn returns_none_for_an_empty_or_whitespace_query() {
        assert_eq!(title_of(""), None);
        assert_eq!(title_of("   "), None);
    }

    #[test]
    fn a_generic_genre_word_does_not_hijack_the_queue() {
        // The exact regression the threshold is set for: at 0.5 this matched an
        // unrelated track.
        assert_eq!(title_of("lofi"), None);
    }

    #[test]
    fn handles_an_empty_library() {
        assert!(Matcher::new(&[]).best("anything").is_none());
    }

    #[test]
    fn keeps_the_threshold_conservative() {
        const { assert!(MATCH_THRESHOLD <= 0.4) };
    }

    #[test]
    fn search_text_lowercases_and_joins() {
        assert_eq!(search_text_for("Title", "Artist", "Album"), "title artist album");
    }

    #[test]
    fn scores_match_fuse() {
        // Reference output of fuse.js 7.5.0 with match.ts's options over the
        // same eight tracks: (query, top result id, score).
        let cases: &[(&str, Option<(&str, f64)>)] = &[
            ("Ambent Gold", Some(("a1", 0.34237139291281704))),
            ("Twinkle", Some(("b1", 0.045603691595129614))),
            ("Cofee Nook", Some(("c1", 0.3572728381519289))),
            ("Butterfly Effec", Some(("d1", 0.045603691595129614))),
            ("Harris Heller", Some(("a1", 0.045603691595129614))),
            ("Metropolis", Some(("b2", 0.07345138681571149))),
            ("lofi", None),
            ("zzzzqqqqxxxx", None),
            ("Never Gonna Give You Up", None),
            ("Bohemian Rhapsody", None),
            ("shape of you", None),
            ("neon in the metropolis harris heller electric", Some(("b2", 0.0000012104710445592926))),
            ("ambient gold harris heller gold", Some(("a1", 1.0066042988583822e-7))),
            ("doin fine", Some(("d2", 0.3745015316914943))),
            ("no ones hero", Some(("a2", 0.3628223664487241))),
            ("rian", None),
            ("electirc", Some(("b1", 0.5381200623918986))),
            ("cofeee nok prime harris heller and more words here", Some(("c1", 0.8195056188007956))),
        ];
        let ts = tracks();
        let m = Matcher::new(&ts);
        for (q, want) in cases {
            let got = m.fuse_top(q);
            match (got, want) {
                (None, None) => {}
                (Some((t, s)), Some((id, score))) => {
                    assert_eq!(t.id, *id, "{q}");
                    assert!((s - score).abs() <= score * 1e-9, "{q}: {s} != {score}");
                }
                _ => panic!("{q}: got {got:?}, want {want:?}"),
            }
        }
        // Above the threshold after the norm: Fuse returned it, match.ts did not.
        assert!(m.best("electirc").is_none());
    }

    #[test]
    fn long_queries_are_chunked_like_fuse() {
        let ts = tracks();
        let m = Matcher::new(&ts);
        // Over 32 UTF-16 units: split into chunks, scores averaged.
        assert!(m.best("neon in the metropolis harris heller electric").is_some());
    }

    #[test]
    fn filter_is_a_case_insensitive_substring() {
        let ts = tracks();
        assert_eq!(filter(&ts, "").len(), ts.len());
        assert_eq!(filter(&ts, "  GOLD ").len(), 2);
        assert!(filter(&ts, "ambent").is_empty()); // not fuzzy
    }
}
