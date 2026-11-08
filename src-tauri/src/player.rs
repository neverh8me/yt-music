//! Playback queue engine. Owns the queue, shuffle/repeat, autoplay radio, history and
//! prefetching; the web layer only plays the URL of `current` and reports events
//! (ended, prev with position). Every change is persisted and broadcast as a
//! `player-state` event.
use crate::models::Track;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// Where the queue came from ("Next from: <title>").
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Context {
    pub kind: String,
    pub id: Option<serde_json::Value>,
    pub title: Option<String>,
}

/// A queue entry; `uid` keeps identity when the same track is queued twice.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Item {
    pub uid: u64,
    pub track: Track,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Queue {
    pub items: Vec<Item>,
    /// Unshuffled order, kept while shuffle is on.
    pub original: Option<Vec<Item>>,
    pub index: Option<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub autoplay: bool,
    pub context: Option<Context>,
    /// Bumped whenever playback should (re)start from the beginning of `current`.
    pub play_seq: u64,
    next_uid: u64,
    rng: u64,
}

/// What the UI should do after `next`/`prev`.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    /// `current` changed (or was restarted) — load and play it.
    Play,
    /// Seek current to 0 and keep playing.
    Restart,
    /// End of queue — pause.
    Stop,
}

#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub index: Option<usize>,
    pub current: Option<Item>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub autoplay: bool,
    pub context: Option<Context>,
    pub play_seq: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self { autoplay: true, ..Default::default() }
    }

    fn seed(&mut self) {
        if self.rng == 0 {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15);
            self.rng = t | 1;
        }
    }

    /// xorshift64* — plenty for shuffling a playlist, no extra dependency.
    fn rand_below(&mut self, n: usize) -> usize {
        self.seed();
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
    }

    fn shuffle_in_place(&mut self, v: &mut [Item]) {
        for i in (1..v.len()).rev() {
            let j = self.rand_below(i + 1);
            v.swap(i, j);
        }
    }

    fn wrap(&mut self, tracks: Vec<Track>) -> Vec<Item> {
        tracks
            .into_iter()
            .map(|track| {
                self.next_uid += 1;
                Item { uid: self.next_uid, track }
            })
            .collect()
    }

    pub fn current(&self) -> Option<&Item> {
        self.index.and_then(|i| self.items.get(i))
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            items: self.items.clone(),
            index: self.index,
            current: self.current().cloned(),
            shuffle: self.shuffle,
            repeat: self.repeat,
            autoplay: self.autoplay,
            context: self.context.clone(),
            play_seq: self.play_seq,
        }
    }

    fn started(&mut self) {
        self.play_seq += 1;
    }

    pub fn play_list(&mut self, tracks: Vec<Track>, start: usize, context: Option<Context>) {
        if tracks.is_empty() {
            return;
        }
        let start = start.min(tracks.len() - 1);
        let mut items = self.wrap(tracks);
        self.context = context;
        if self.shuffle {
            self.original = Some(items.clone());
            let first = items.remove(start);
            self.shuffle_in_place(&mut items);
            items.insert(0, first);
            self.items = items;
            self.index = Some(0);
        } else {
            self.original = None;
            self.items = items;
            self.index = Some(start);
        }
        self.started();
    }

    pub fn play_next(&mut self, tracks: Vec<Track>) {
        let items = self.wrap(tracks);
        match self.index {
            None => {
                self.items = items;
                self.index = Some(0);
                self.started();
            }
            Some(i) => {
                if let Some(o) = &mut self.original {
                    o.extend(items.iter().cloned());
                }
                self.items.splice(i + 1..i + 1, items);
            }
        }
    }

    pub fn add_to_queue(&mut self, tracks: Vec<Track>) {
        let items = self.wrap(tracks);
        if let Some(o) = &mut self.original {
            o.extend(items.iter().cloned());
        }
        let was_empty = self.index.is_none();
        self.items.extend(items);
        if was_empty && !self.items.is_empty() {
            self.index = Some(0);
            self.started();
        }
    }

    pub fn remove(&mut self, i: usize) {
        let Some(cur) = self.index else { return };
        if i == cur || i >= self.items.len() {
            return;
        }
        let uid = self.items.remove(i).uid;
        if let Some(o) = &mut self.original {
            o.retain(|x| x.uid != uid);
        }
        if i < cur {
            self.index = Some(cur - 1);
        }
    }

    pub fn jump(&mut self, i: usize) {
        if i < self.items.len() {
            self.index = Some(i);
            self.started();
        }
    }

    /// Move forward. `None` means the queue is exhausted and autoplay may extend it.
    pub fn next(&mut self, auto: bool) -> Option<Step> {
        let cur = self.index?;
        if auto && self.repeat == Repeat::One {
            return Some(Step::Restart);
        }
        if cur + 1 < self.items.len() {
            self.index = Some(cur + 1);
            self.started();
            return Some(Step::Play);
        }
        if self.repeat == Repeat::All {
            self.index = Some(0);
            self.started();
            return Some(Step::Play);
        }
        None
    }

    pub fn prev(&mut self, position_secs: f64) -> Step {
        match self.index {
            Some(cur) if position_secs <= 3.0 && cur > 0 => {
                self.index = Some(cur - 1);
                self.started();
                Step::Play
            }
            Some(_) => Step::Restart,
            None => Step::Stop,
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        let cur_uid = self.current().map(|c| c.uid);
        if self.shuffle {
            self.original = Some(self.items.clone());
            if let Some(i) = self.index {
                let mut upcoming = self.items.split_off(i + 1);
                self.shuffle_in_place(&mut upcoming);
                self.items.extend(upcoming);
            }
        } else if let Some(orig) = self.original.take() {
            self.items = orig;
            self.index = cur_uid.and_then(|u| self.items.iter().position(|x| x.uid == u)).or(self.index.map(|_| 0));
        }
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
    }

    /// Append radio tracks, skipping ones already queued. Returns how many were added.
    pub fn extend_radio(&mut self, tracks: Vec<Track>) -> usize {
        let have: std::collections::HashSet<_> = self.items.iter().map(|x| x.track.id.clone()).collect();
        let fresh: Vec<Track> = tracks.into_iter().filter(|t| !have.contains(&t.id)).take(25).collect();
        let n = fresh.len();
        let items = self.wrap(fresh);
        if let Some(o) = &mut self.original {
            o.extend(items.iter().cloned());
        }
        self.items.extend(items);
        n
    }

    /// Queue to persist across restarts (bounded).
    pub fn for_storage(&self) -> Queue {
        let mut q = self.clone();
        if q.items.len() > 500 {
            let start = q.index.unwrap_or(0).saturating_sub(100);
            q.items = q.items.split_off(start);
            q.items.truncate(500);
            q.index = q.index.map(|i| i - start);
            q.original = None;
            q.shuffle = false;
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(n: usize) -> Vec<Track> {
        (0..n)
            .map(|i| Track { id: format!("t{i}"), title: format!("T{i}"), artists: vec![], album: None, duration: None, cover: None, plays: None })
            .collect()
    }
    fn ids(q: &Queue) -> Vec<String> {
        q.items.iter().map(|x| x.track.id.clone()).collect()
    }

    #[test]
    fn basic_navigation() {
        let mut q = Queue::new();
        q.play_list(tracks(3), 1, None);
        assert_eq!(q.current().unwrap().track.id, "t1");
        assert_eq!(q.next(false), Some(Step::Play));
        assert_eq!(q.current().unwrap().track.id, "t2");
        assert_eq!(q.next(true), None, "end of queue");
        assert_eq!(q.prev(1.0), Step::Play);
        assert_eq!(q.current().unwrap().track.id, "t1");
        assert_eq!(q.prev(10.0), Step::Restart, "late prev restarts the track");
    }

    #[test]
    fn repeat_modes() {
        let mut q = Queue::new();
        q.play_list(tracks(2), 1, None);
        q.cycle_repeat(); // all
        assert_eq!(q.next(true), Some(Step::Play));
        assert_eq!(q.index, Some(0));
        q.cycle_repeat(); // one
        assert_eq!(q.next(true), Some(Step::Restart));
        assert_eq!(q.next(false), Some(Step::Play), "manual skip still moves on repeat-one");
    }

    #[test]
    fn shuffle_keeps_current_and_restores_order() {
        let mut q = Queue::new();
        q.play_list(tracks(20), 5, None);
        q.toggle_shuffle();
        assert_eq!(q.current().unwrap().track.id, "t5");
        assert_eq!(q.items.len(), 20);
        let mut sorted = ids(&q);
        sorted.sort();
        let mut expect = ids(&{
            let mut x = Queue::new();
            x.play_list(tracks(20), 0, None);
            x
        });
        expect.sort();
        assert_eq!(sorted, expect, "same tracks after shuffle");
        q.next(false);
        let playing = q.current().unwrap().uid;
        q.toggle_shuffle();
        assert_eq!(ids(&q)[0], "t0", "original order restored");
        assert_eq!(q.current().unwrap().uid, playing, "still on the same track");
    }

    #[test]
    fn queue_edits() {
        let mut q = Queue::new();
        q.play_list(tracks(3), 1, None);
        q.play_next(tracks(1));
        assert_eq!(ids(&q), ["t0", "t1", "t0", "t2"]);
        q.remove(0);
        assert_eq!(q.index, Some(0));
        assert_eq!(q.current().unwrap().track.id, "t1");
        assert_eq!(q.extend_radio(tracks(5)), 2, "t0..t2 already queued, only t3/t4 added");
    }
}
