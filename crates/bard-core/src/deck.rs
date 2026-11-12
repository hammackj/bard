//! A shuffle deck: every track is dealt once before any is dealt again.
//!
//! This is the thing random-with-a-recent-window cannot do. With 585 tracks and
//! a 50-track window, a "random" pick can still serve the same track twice in an
//! evening while leaving others untouched for weeks. Dealing from a shuffled
//! deck and only reshuffling once it is exhausted guarantees full coverage, and
//! it is the specific complaint people have about shuffle in big players.
//!
//! A port of `src/shared/deck.ts`.

use crate::playlist::{Rng, pick_index, shuffled};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deck {
    /// Undealt ids, in the order they will be dealt.
    pub remaining: Vec<String>,
    /// Ids dealt since the last reshuffle, for reporting progress through it.
    pub dealt: usize,
    /// Last id dealt, so a rebuilt deck does not repeat it across the seam.
    pub last: Option<String>,
}

/// Copies a track gets in each pass.
///
/// Weighting inside the deck rather than picking loved tracks more often at
/// random is what keeps the guarantee intact: every track still appears in
/// every pass, loved ones simply appear more than once.
pub type WeightOf<'a> = &'a dyn Fn(&str) -> f64;

fn expand(ids: &[String], weight_of: Option<WeightOf>) -> Vec<String> {
    let Some(w) = weight_of else { return ids.to_vec() };
    let mut out = Vec::new();
    for id in ids {
        let copies = w(id).floor().max(1.0) as usize;
        for _ in 0..copies {
            out.push(id.clone());
        }
    }
    out
}

/// Arranges a weighted deck so no two identical cards sit next to each other.
///
/// Built rather than patched: an earlier version swapped randomly a few times
/// and gave up, which left adjacent duplicates whenever a loved track was a
/// large share of a small library. Placing the most frequent card into every
/// other slot first is correct by construction whenever it is possible at all
/// (that is, when no card exceeds half the deck, rounded up).
fn spread(order: &[String], rng: Rng) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut distinct: Vec<String> = Vec::new();
    for id in order {
        let n = counts.entry(id).or_insert(0);
        if *n == 0 {
            distinct.push(id.clone());
        }
        *n += 1;
    }

    // Shuffle the distinct ids so equal-frequency cards are not always in the
    // same order, then place the most frequent first. (Stable sort, as JS.)
    let mut distinct = shuffled(&distinct, rng);
    distinct.sort_by(|a, b| counts[b.as_str()].cmp(&counts[a.as_str()]));

    let mut out: Vec<Option<String>> = vec![None; order.len()];
    let mut slot = 0;
    for id in &distinct {
        for _ in 0..counts[id.as_str()] {
            if slot >= out.len() {
                slot = 1; // wrap to the odd slots
            }
            out[slot] = Some(id.clone());
            slot += 2;
        }
    }
    out.into_iter().flatten().collect()
}

pub fn new_deck(ids: &[String], rng: Rng, avoid_first: Option<&str>, weight_of: Option<WeightOf>) -> Deck {
    let mut remaining = if weight_of.is_some() {
        let expanded = shuffled(&expand(ids, weight_of), rng);
        spread(&expanded, rng)
    } else {
        shuffled(ids, rng)
    };
    // Avoid dealing the track that just played as the first of a new deck: the
    // seam between decks is the one place a repeat can still feel wrong.
    if let Some(avoid) = avoid_first
        && remaining.first().is_some_and(|f| f == avoid)
    {
        avoid_seam(&mut remaining, avoid, rng);
    }
    Deck { remaining, dealt: 0, last: None }
}

/// Brings a different track to the front, keeping every card. A weighted
/// deck holds several copies of a track, so "any other slot" may hold the
/// same one. Preferred: a rotation that starts on another track and leaves
/// no two equal cards side by side (a rotation keeps the deck's spacing,
/// only joining its ends); then a swap with a slot holding another track
/// that leaves none; then any such swap. With one distinct track there is
/// nothing else to deal, and the deck stays as it is.
fn avoid_seam(deck: &mut Vec<String>, avoid: &str, rng: Rng) {
    let n = deck.len();
    // Equal neighbours, counted once; a rotation or a swap changes only a
    // few pairs, so each candidate is checked in O(1), not by rebuilding.
    let eq = |d: &[String], i: usize, j: usize| usize::from(d[i] == d[j]);
    let pairs: usize = (1..n).map(|i| eq(deck, i - 1, i)).sum();
    // Rotating left by k splits the pair (k-1, k) and joins (n-1, 0).
    let rotations: Vec<usize> =
        (1..n).filter(|&k| deck[k] != avoid && pairs - eq(deck, k - 1, k) + eq(deck, n - 1, 0) == 0).collect();
    if !rotations.is_empty() {
        deck.rotate_left(rotations[pick_index(rng, rotations.len())]);
        return;
    }
    // Swapping 0 and j changes only the pairs touching either slot: count
    // those before and after, reading slot i from where the swap puts it.
    let after_swap = |j: usize| {
        let mut touched: Vec<(usize, usize)> = vec![(0, 1), (j - 1, j)];
        if j + 1 < n {
            touched.push((j, j + 1));
        }
        touched.sort_unstable();
        touched.dedup();
        let from = |i: usize| {
            if i == 0 {
                j
            } else if i == j {
                0
            } else {
                i
            }
        };
        let count = |at: &dyn Fn(usize) -> usize| touched.iter().filter(|&&(a, b)| deck[at(a)] == deck[at(b)]).count();
        pairs - count(&|i| i) + count(&from)
    };
    let differs: Vec<usize> = (1..n).filter(|&j| deck[j] != avoid).collect();
    let clean: Vec<usize> = differs.iter().copied().filter(|&j| after_swap(j) == 0).collect();
    let from = if clean.is_empty() { &differs } else { &clean };
    if !from.is_empty() {
        deck.swap(0, from[pick_index(rng, from.len())]);
    }
}

/// Deals the next id, rebuilding the deck when it runs out.
///
/// `available` is passed on every draw so the deck follows a library that has
/// been rescanned: ids that no longer exist are dropped rather than dealt.
pub fn draw(
    deck: Option<&Deck>,
    available: &[String],
    rng: Rng,
    avoid_first: Option<&str>,
    weight_of: Option<WeightOf>,
) -> (Option<String>, Deck) {
    if available.is_empty() {
        return (None, Deck::default());
    }

    let valid: HashSet<&str> = available.iter().map(String::as_str).collect();
    // The seam is guarded by whatever was dealt last, whether the caller told us
    // or not -- relying on the caller meant a rebuild could repeat a card.
    let guard = avoid_first.or_else(|| deck.and_then(|d| d.last.as_deref()));

    let mut current = match deck {
        Some(d) if !d.remaining.is_empty() => Deck {
            remaining: d.remaining.iter().filter(|id| valid.contains(id.as_str())).cloned().collect(),
            ..d.clone()
        },
        _ => new_deck(available, rng, guard, weight_of),
    };
    if current.remaining.is_empty() {
        current = new_deck(available, rng, guard, weight_of);
    }

    let mut rest = current.remaining.into_iter();
    let id = rest.next();
    let last = id.clone().or_else(|| deck.and_then(|d| d.last.clone()));
    (id, Deck { remaining: rest.collect(), dealt: current.dealt + 1, last })
}

/// How far through the current deck we are, 0..1.
pub fn deck_progress(deck: Option<&Deck>, total: usize) -> f64 {
    match deck {
        Some(d) if total > 0 => (d.dealt as f64 / total as f64).min(1.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::math_random;

    fn ids() -> Vec<String> {
        ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect()
    }
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    /// Deterministic rng cycling through the given values.
    fn seq(v: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut i = 0;
        move || {
            let x = v[i % v.len()];
            i += 1;
            x
        }
    }

    #[test]
    fn deals_every_track_before_repeating_any() {
        // The whole point: a rolling random window cannot guarantee this.
        let ids = ids();
        let mut rng = seq(&[0.1, 0.7, 0.3, 0.9]);
        let mut deck: Option<Deck> = None;
        let mut dealt = Vec::new();
        for _ in 0..ids.len() {
            let (id, d) = draw(deck.as_ref(), &ids, &mut rng, None, None);
            deck = Some(d);
            dealt.push(id.unwrap());
        }
        dealt.sort();
        assert_eq!(dealt, ids);
    }

    #[test]
    fn reshuffles_once_exhausted_and_keeps_dealing() {
        let ids = ids();
        let mut rng = seq(&[0.4, 0.8, 0.2]);
        let mut deck: Option<Deck> = None;
        let mut dealt = Vec::new();
        for _ in 0..15 {
            let (id, d) = draw(deck.as_ref(), &ids, &mut rng, None, None);
            deck = Some(d);
            dealt.push(id.unwrap());
        }
        assert_eq!(dealt.len(), 15);
        assert!(dealt.iter().all(|d| ids.contains(d)));
        // Each full pass should still contain every track exactly once.
        for pass in dealt.chunks(5) {
            assert_eq!(pass.iter().collect::<HashSet<_>>().len(), 5);
        }
    }

    #[test]
    fn avoids_repeating_the_previous_track_across_the_seam() {
        // rng returns 0, so an unguarded shuffle would deal the same first card.
        let ids = ids();
        let deck = new_deck(&ids, &mut || 0.0, Some("a"), None);
        assert_ne!(deck.remaining[0], "a");
    }

    /// A weighted deck has several copies of a track: the seam swaps in a
    /// different track, not another copy of the one just played -- for
    /// every rng value, keeping each track's copies, and without leaving two
    /// equal cards side by side where it need not.
    #[test]
    fn a_weighted_seam_swaps_in_a_different_track() {
        let counts = |d: &[String]| {
            let mut m: HashMap<String, usize> = HashMap::new();
            d.iter().for_each(|id| *m.entry(id.clone()).or_default() += 1);
            m
        };
        let adjacent = |d: &[String]| d.windows(2).any(|w| w[0] == w[1]);
        // The reviewer's: a twice, b and c once, rng always 0.5.
        let abc = strs(&["a", "b", "c"]);
        let w2 = |id: &str| if id == "a" { 2.0 } else { 1.0 };
        let deck = new_deck(&abc, &mut || 0.5, Some("a"), Some(&w2));
        assert_ne!(deck.remaining[0], "a", "{:?}", deck.remaining);

        let cases: [(&[&str], fn(&str) -> f64); 3] = [
            (&["a", "b", "c"], |id| if id == "a" { 2.0 } else { 1.0 }),
            (&["a", "b", "c", "d", "e"], |id| if id == "a" { 3.0 } else { 1.0 }),
            (&["a", "b"], |id| if id == "a" { 3.0 } else { 1.0 }),
        ];
        for (lib, weight) in cases {
            let lib = strs(lib);
            for step in 0..20 {
                let r = step as f64 / 20.;
                let plain = new_deck(&lib, &mut || r, None, Some(&weight));
                let deck = new_deck(&lib, &mut || r, Some("a"), Some(&weight));
                assert_ne!(deck.remaining[0], "a", "{lib:?} r={r}: {:?}", deck.remaining);
                assert_eq!(counts(&deck.remaining), counts(&plain.remaining), "copies kept");
                if !adjacent(&plain.remaining) && lib.len() > 2 {
                    assert!(!adjacent(&deck.remaining), "{lib:?} r={r}: {:?}", deck.remaining);
                }
            }
        }

        // Only one distinct track: nothing else to deal.
        let one = strs(&["a"]);
        let deck = new_deck(&one, &mut || 0.5, Some("a"), Some(&|_: &str| 3.0));
        assert!(deck.remaining.iter().all(|id| id == "a") && !deck.remaining.is_empty());
    }

    #[test]
    fn drops_ids_that_no_longer_exist_after_a_rescan() {
        let stale = Deck { remaining: strs(&["a", "gone", "b"]), dealt: 0, last: None };
        let (id, d) = draw(Some(&stale), &strs(&["a", "b"]), &mut seq(&[0.5]), None, None);
        assert!(["a", "b"].contains(&id.unwrap().as_str()));
        assert!(!d.remaining.contains(&"gone".to_string()));
    }

    #[test]
    fn returns_none_for_an_empty_library() {
        assert!(draw(None, &[], &mut seq(&[0.5]), None, None).0.is_none());
    }

    #[test]
    fn handles_a_single_track_library() {
        let only = strs(&["only"]);
        let (id, d) = draw(None, &only, &mut seq(&[0.5]), None, None);
        assert_eq!(id.as_deref(), Some("only"));
        assert_eq!(draw(Some(&d), &only, &mut seq(&[0.5]), None, None).0.as_deref(), Some("only"));
    }

    #[test]
    fn never_stalls_585_tracks_for_many_passes() {
        // The "endless" promise. Deals well past the library size and checks that
        // it keeps producing tracks and that every pass is a full permutation.
        let big: Vec<String> = (0..585).map(|i| format!("t{i}")).collect();
        let mut deck: Option<Deck> = None;
        let mut dealt = Vec::new();
        for _ in 0..585 * 4 {
            let (id, d) = draw(deck.as_ref(), &big, &mut math_random, None, None);
            deck = Some(d);
            dealt.push(id.expect("never stalls"));
        }
        assert_eq!(dealt.len(), 2340);
        for pass in dealt.chunks(585) {
            assert_eq!(pass.iter().collect::<HashSet<_>>().len(), 585);
        }
    }

    #[test]
    fn keeps_dealing_when_the_library_shrinks_mid_deck() {
        let mut deck = Some(draw(None, &strs(&["a", "b", "c", "d"]), &mut math_random, None, None).1);
        for _ in 0..20 {
            let (id, d) = draw(deck.as_ref(), &strs(&["a", "b"]), &mut math_random, None, None);
            assert!(["a", "b"].contains(&id.unwrap().as_str()));
            deck = Some(d);
        }
    }

    #[test]
    fn loved_tracks_dealt_more_often_but_everything_still_dealt() {
        let lib: Vec<String> = (0..50).map(|i| format!("t{i}")).collect();
        let weight = |id: &str| if id == "t0" || id == "t1" { 3.0 } else { 1.0 };
        let mut deck: Option<Deck> = None;
        let mut counts: HashMap<String, usize> = HashMap::new();
        // One full pass: 48 singles + 2 tracks x3 = 54 cards.
        for _ in 0..54 {
            let (id, d) = draw(deck.as_ref(), &lib, &mut math_random, None, Some(&weight));
            deck = Some(d);
            *counts.entry(id.unwrap()).or_default() += 1;
        }
        // Nothing is starved: every track still appears in the pass.
        assert_eq!(counts.len(), 50);
        assert_eq!(counts["t0"], 3);
        assert_eq!(counts["t1"], 3);
        assert_eq!(counts["t2"], 1);
    }

    #[test]
    fn does_not_deal_a_loved_track_twice_in_a_row() {
        let lib = strs(&["a", "b", "c", "d", "e", "f"]);
        let weight = |id: &str| if id == "a" { 3.0 } else { 1.0 };
        let mut deck: Option<Deck> = None;
        let mut previous: Option<String> = None;
        for _ in 0..40 {
            let (id, d) = draw(deck.as_ref(), &lib, &mut math_random, None, Some(&weight));
            deck = Some(d);
            assert_ne!(id, previous);
            previous = id;
        }
    }

    #[test]
    fn weight_one_behaves_like_an_unweighted_deck() {
        let lib = strs(&["a", "b", "c"]);
        let mut deck: Option<Deck> = None;
        let mut seen = HashSet::new();
        for _ in 0..3 {
            let (id, d) = draw(deck.as_ref(), &lib, &mut math_random, None, Some(&|_| 1.0));
            deck = Some(d);
            seen.insert(id.unwrap());
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn reports_progress_through_the_deck() {
        assert_eq!(deck_progress(None, 10), 0.0);
        let d = |dealt| Deck { remaining: vec![], dealt, last: None };
        assert_eq!(deck_progress(Some(&d(5)), 10), 0.5);
        assert_eq!(deck_progress(Some(&d(99)), 10), 1.0);
    }
}
