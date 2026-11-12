//! Play history arithmetic. A port of `src/shared/history.ts`.
//!
//! Small, but the off-by-one here is the kind that only shows up as "back went
//! to the wrong song" three tracks later, so it lives apart from the queue's
//! mutable state where it can actually be tested.
//!
//! The invariant everything below assumes: the last entry of `played` is the
//! track playing right now. It is pushed when playback starts, not when it
//! finishes.

/// Anything with a stable identity; the queue passes its QueueItems.
pub trait HasId {
    fn id(&self) -> &str;
}

impl HasId for crate::types::QueueItem {
    fn id(&self) -> &str {
        &self.id
    }
}

/// How far into a track "previous" restarts it instead of stepping back.
pub const RESTART_THRESHOLD_SEC: f64 = 3.0;

/// Most history to keep. Bounded so a long session cannot grow without limit.
pub const PLAYED_LIMIT: usize = 200;

pub fn push_played<T: Clone>(played: &[T], item: T) -> Vec<T> {
    let mut out = played.to_vec();
    out.push(item);
    let excess = out.len().saturating_sub(PLAYED_LIMIT);
    out.drain(..excess);
    out
}

/// The recent-tracks window shuffle refuses to repeat from.
///
/// `limit` is the avoid-repeat setting, and 0 has to mean "avoid nothing".
/// Expressing that as `slice(-limit)` in JavaScript did the opposite:
/// `slice(-0)` is `slice(0)`, which returns the whole array rather than an
/// empty one, so a setting of 0 grew the window without bound and excluded
/// every track that had ever played -- the exact inverse of what was asked for.
///
/// The id is moved to the newest position rather than duplicated, so replaying
/// a track refreshes its place in the window instead of consuming two slots.
pub fn remember_recent(recent: &[String], id: &str, limit: i64) -> Vec<String> {
    if limit <= 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = recent.iter().filter(|x| *x != id).cloned().collect();
    out.push(id.to_string());
    let excess = out.len().saturating_sub(limit as usize);
    out.drain(..excess);
    out
}

/// What pressing "previous" should do.
///
/// `Restart` covers both the common case (a few seconds in, you meant "play
/// this again") and the boundary (nothing behind us) -- in both, replaying the
/// current track is better than doing nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum BackStep<T> {
    Restart,
    Back { prior: T, played: Vec<T> },
}

pub fn step_back<T: Clone>(played: &[T], position_sec: f64) -> BackStep<T> {
    if position_sec > RESTART_THRESHOLD_SEC {
        return BackStep::Restart;
    }
    // The last entry is what is playing now, so the one before it is len-2.
    if played.len() < 2 {
        return BackStep::Restart;
    }
    BackStep::Back {
        prior: played[played.len() - 2].clone(),
        played: played[..played.len() - 1].to_vec(),
    }
}

/// Queue and history after repeat-all wraps around.
///
/// The subtlety: `played` ends with the track that just finished, so re-queuing
/// it wholesale plays that track twice across the seam. It is dropped here and
/// kept as the sole history entry, which both starts the next lap's history
/// cleanly and leaves one step of "back" still working across the wrap.
pub fn wrap_for_repeat_all<T: Clone + HasId>(played: &[T], current: Option<&T>) -> (Vec<T>, Vec<T>) {
    let current_id = current.map(|c| c.id()).or_else(|| played.last().map(|p| p.id()));
    let queue = played.iter().filter(|i| Some(i.id()) != current_id).cloned().collect();
    let played = current.map(|c| vec![c.clone()]).unwrap_or_default();
    (queue, played)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Item(String);
    impl HasId for Item {
        fn id(&self) -> &str {
            &self.0
        }
    }
    fn item(id: &str) -> Item {
        Item(id.into())
    }
    fn ids(xs: &[Item]) -> Vec<&str> {
        xs.iter().map(|x| x.0.as_str()).collect()
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn push_played_appends_newest_last() {
        assert_eq!(ids(&push_played(&[item("a")], item("b"))), ["a", "b"]);
    }

    #[test]
    fn push_played_keeps_history_bounded() {
        let mut played = Vec::new();
        for i in 0..PLAYED_LIMIT + 25 {
            played = push_played(&played, item(&format!("t{i}")));
        }
        assert_eq!(played.len(), PLAYED_LIMIT);
        assert_eq!(played[0].0, "t25");
        assert_eq!(played.last().unwrap().0, format!("t{}", PLAYED_LIMIT + 24));
    }

    fn abc() -> Vec<Item> {
        vec![item("a"), item("b"), item("c")] // c is playing
    }

    #[test]
    fn step_back_restarts_a_few_seconds_in() {
        assert_eq!(step_back(&abc(), 10.0), BackStep::Restart);
    }

    #[test]
    fn step_back_steps_back_when_barely_in() {
        let BackStep::Back { prior, played } = step_back(&abc(), 1.0) else { panic!() };
        // The one before what is playing, not what is playing.
        assert_eq!(prior.0, "b");
        // History drops the track we stepped away from, so b is now the tail.
        assert_eq!(ids(&played), ["a", "b"]);
    }

    #[test]
    fn step_back_walks_back_one_track_per_press() {
        let mut history = abc();
        let mut visited = Vec::new();
        for _ in 0..2 {
            let BackStep::Back { prior, played } = step_back(&history, 0.0) else { break };
            visited.push(prior.0);
            history = played;
        }
        assert_eq!(visited, ["b", "a"]);
    }

    #[test]
    fn step_back_restarts_at_the_start_of_history() {
        assert_eq!(step_back(&[item("a")], 0.0), BackStep::Restart);
        assert_eq!(step_back::<Item>(&[], 0.0), BackStep::Restart);
    }

    #[test]
    fn wrap_does_not_replay_the_track_that_just_finished() {
        let (queue, _) = wrap_for_repeat_all(&abc(), Some(&item("c")));
        assert_eq!(ids(&queue), ["a", "b"]);
    }

    #[test]
    fn wrap_carries_the_current_track_into_the_next_lap() {
        let (_, played) = wrap_for_repeat_all(&[item("a"), item("b")], Some(&item("b")));
        assert_eq!(ids(&played), ["b"]);
    }

    #[test]
    fn wrap_loops_without_duplicating_across_two_wraps() {
        let (q1, p1) = wrap_for_repeat_all(&abc(), Some(&item("c")));
        assert_eq!(ids(&q1), ["a", "b"]);
        // Lap two plays a then b, and b finishes.
        let played = push_played(&push_played(&p1, item("a")), item("b"));
        let (q2, _) = wrap_for_repeat_all(&played, Some(&item("b")));
        assert_eq!(ids(&q2), ["c", "a"]);
        let unique: std::collections::HashSet<_> = ids(&q2).into_iter().collect();
        assert_eq!(unique.len(), q2.len());
    }

    #[test]
    fn wrap_falls_back_to_the_tail_when_nothing_is_playing() {
        let (queue, _) = wrap_for_repeat_all(&[item("a"), item("b")], None);
        assert_eq!(ids(&queue), ["a"]);
    }

    // The zero case is the bug worth a test: slice(-0) is slice(0).
    #[test]
    fn remember_recent_keeps_nothing_at_limit_0() {
        assert!(remember_recent(&s(&["a", "b", "c"]), "d", 0).is_empty());
    }

    #[test]
    fn remember_recent_keeps_nothing_at_a_negative_limit() {
        assert!(remember_recent(&s(&["a", "b"]), "c", -5).is_empty());
    }

    #[test]
    fn remember_recent_appends_up_to_the_limit() {
        assert_eq!(remember_recent(&s(&["a"]), "b", 3), ["a", "b"]);
    }

    #[test]
    fn remember_recent_drops_the_oldest() {
        assert_eq!(remember_recent(&s(&["a", "b", "c"]), "d", 3), ["b", "c", "d"]);
    }

    #[test]
    fn remember_recent_moves_a_repeat_rather_than_duplicating() {
        assert_eq!(remember_recent(&s(&["a", "b", "c"]), "b", 3), ["a", "c", "b"]);
    }

    #[test]
    fn remember_recent_holds_one_at_limit_1() {
        assert_eq!(remember_recent(&s(&["a"]), "b", 1), ["b"]);
    }
}
