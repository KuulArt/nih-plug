//! Coalesces GUI parameter edits made while the host has stopped calling `process()`. REAPER's FX
//! bypass neither processes nor flushes, so queuing every gesture and value would overflow the
//! output event queue and drop the newest values, and the replay after unbypass would end on stale
//! ones. Instead each parameter keeps at most one pending bundle (begin, latest value, end) that a
//! single marker in the output queue stands for. The marker keeps the bundle's place in the queue
//! order, and once a bundle is pending every later edit of that parameter joins it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// A marker for this parameter sits in the output queue.
const QUEUED: u32 = 1 << 0;
/// The bundle opens a gesture.
const BEGIN: u32 = 1 << 1;
/// The bundle carries a value.
const VALUE: u32 = 1 << 2;
/// The bundle closes a gesture.
const END: u32 = 1 << 3;

/// One GUI parameter edit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Begin,
    Set(f64),
    End,
}

/// Where an edit goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Queue the edit's own event, exactly as when nothing is stalled.
    Direct,
    /// The edit joined the parameter's bundle; push a marker if `push_marker`.
    Coalesced { push_marker: bool },
}

/// A drained bundle, to be written to the host in this order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bundle {
    pub begin: bool,
    pub value: Option<f64>,
    pub end: bool,
}

#[derive(Default)]
struct Slot {
    /// The latest value's `f64` bits, valid while `VALUE` is set.
    value: AtomicU64,
    flags: AtomicU32,
}

/// The pending bundles, one slot per parameter, all allocated up front.
pub struct StalledEdits {
    slots: HashMap<u32, Slot>,
}

impl StalledEdits {
    pub fn new(param_hashes: impl IntoIterator<Item = u32>) -> Self {
        Self {
            slots: param_hashes
                .into_iter()
                .map(|hash| (hash, Slot::default()))
                .collect(),
        }
    }

    /// Records a GUI edit. Edits go into the bundle while `stalled`, and also while a bundle is
    /// pending so they can't overtake it.
    pub fn record(&self, param_hash: u32, edit: Edit, stalled: bool) -> Route {
        let Some(slot) = self.slots.get(&param_hash) else {
            return Route::Direct;
        };
        if !stalled && slot.flags.load(Ordering::Acquire) == 0 {
            return Route::Direct;
        }

        if let Edit::Set(value) = edit {
            slot.value.store(value.to_bits(), Ordering::Relaxed);
        }
        let mut previous = slot.flags.load(Ordering::Acquire);
        while let Err(actual) = slot.flags.compare_exchange_weak(
            previous,
            apply(previous, edit) | QUEUED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            previous = actual;
        }

        Route::Coalesced {
            push_marker: previous & QUEUED == 0,
        }
    }

    /// The marker could not be queued. The bundle stays and the next edit queues a marker again.
    pub fn marker_dropped(&self, param_hash: u32) {
        if let Some(slot) = self.slots.get(&param_hash) {
            slot.flags.fetch_and(!QUEUED, Ordering::AcqRel);
        }
    }

    /// The host set this parameter, which is newer than any value still waiting in the bundle, so
    /// the bundle stops carrying one. Gestures stay so they remain balanced.
    pub fn host_set(&self, param_hash: u32) {
        if let Some(slot) = self.slots.get(&param_hash) {
            if slot.flags.load(Ordering::Acquire) & VALUE != 0 {
                slot.flags.fetch_and(!VALUE, Ordering::AcqRel);
            }
        }
    }

    /// A state or preset load replaces every value, so no bundle carries one anymore. Gestures
    /// stay so they remain balanced.
    pub fn drop_values(&self) {
        for slot in self.slots.values() {
            slot.flags.fetch_and(!VALUE, Ordering::AcqRel);
        }
    }

    /// Takes the bundle when its marker is popped from the queue.
    pub fn take(&self, param_hash: u32) -> Bundle {
        let Some(slot) = self.slots.get(&param_hash) else {
            return Bundle::default();
        };
        let flags = slot.flags.swap(0, Ordering::AcqRel);

        Bundle {
            begin: flags & BEGIN != 0,
            value: (flags & VALUE != 0).then(|| f64::from_bits(slot.value.load(Ordering::Relaxed))),
            end: flags & END != 0,
        }
    }
}

/// Folds an edit into a bundle's flags. An end followed by a begin cancels out, which keeps the
/// host's gesture open, so a bundle always reads begin?, value?, end? in that order.
fn apply(flags: u32, edit: Edit) -> u32 {
    match edit {
        Edit::Begin if flags & END != 0 => flags & !END,
        Edit::Begin => flags | BEGIN,
        Edit::Set(_) => flags | VALUE,
        Edit::End => flags | END,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Event {
        Begin(u32),
        Set(u32, f64),
        End(u32),
        Marker(u32),
    }

    /// A GUI and an output queue that the host drains with [`Self::drain()`].
    struct Sim {
        edits: StalledEdits,
        queue: VecDeque<Event>,
    }

    impl Sim {
        fn new(params: u32) -> Self {
            Self {
                edits: StalledEdits::new(0..params),
                queue: VecDeque::new(),
            }
        }

        fn edit(&mut self, hash: u32, edit: Edit, stalled: bool) {
            match self.edits.record(hash, edit, stalled) {
                Route::Direct => self.queue.push_back(match edit {
                    Edit::Begin => Event::Begin(hash),
                    Edit::Set(v) => Event::Set(hash, v),
                    Edit::End => Event::End(hash),
                }),
                Route::Coalesced { push_marker: true } => self.queue.push_back(Event::Marker(hash)),
                Route::Coalesced { push_marker: false } => (),
            }
        }

        fn drain(&mut self) -> Vec<Event> {
            let mut out = Vec::new();
            while let Some(event) = self.queue.pop_front() {
                match event {
                    Event::Marker(hash) => {
                        let bundle = self.edits.take(hash);
                        if bundle.begin {
                            out.push(Event::Begin(hash));
                        }
                        if let Some(v) = bundle.value {
                            out.push(Event::Set(hash, v));
                        }
                        if bundle.end {
                            out.push(Event::End(hash));
                        }
                    }
                    event => out.push(event),
                }
            }
            out
        }
    }

    /// Every gesture the host sees is opened before it is closed, and a value only comes inside
    /// a gesture. `open` is the gesture state before `events`.
    fn assert_balanced(events: &[Event], open: &mut HashMap<u32, bool>) {
        for event in events {
            match *event {
                Event::Begin(h) => assert!(!open.insert(h, true).unwrap_or(false), "{event:?}"),
                Event::End(h) => assert!(open.insert(h, false).unwrap_or(false), "{event:?}"),
                Event::Set(h, _) => assert!(open.get(&h).copied().unwrap_or(false), "{event:?}"),
                Event::Marker(_) => unreachable!(),
            }
        }
    }

    fn last_value(events: &[Event], hash: u32) -> Option<f64> {
        events.iter().rev().find_map(|e| match *e {
            Event::Set(h, v) if h == hash => Some(v),
            _ => None,
        })
    }

    #[test]
    fn live_edits_are_direct() {
        let mut sim = Sim::new(4);
        sim.edit(1, Edit::Begin, false);
        sim.edit(1, Edit::Set(0.5), false);
        sim.edit(1, Edit::End, false);
        assert_eq!(
            sim.drain(),
            [Event::Begin(1), Event::Set(1, 0.5), Event::End(1)]
        );
    }

    #[test]
    fn thousands_of_stalled_writes_drain_to_one_value_per_param() {
        const PARAMS: u32 = 686;
        let mut sim = Sim::new(PARAMS);
        let mut latest = HashMap::new();
        // Many slot switches (begin, set, end per param) and long drags while stalled
        for round in 0..20u32 {
            for hash in 0..PARAMS {
                let v = f64::from(round * PARAMS + hash);
                sim.edit(hash, Edit::Begin, true);
                sim.edit(hash, Edit::Set(v), true);
                sim.edit(hash, Edit::End, true);
                latest.insert(hash, v);
            }
            sim.edit(7, Edit::Begin, true);
            for step in 0..500 {
                let v = f64::from(step) + 0.25;
                sim.edit(7, Edit::Set(v), true);
                latest.insert(7, v);
            }
            sim.edit(7, Edit::End, true);
        }
        assert!(sim.queue.len() <= PARAMS as usize);

        let out = sim.drain();
        assert_balanced(&out, &mut HashMap::new());
        for hash in 0..PARAMS {
            let sets = out
                .iter()
                .filter(|e| matches!(e, Event::Set(h, _) if *h == hash))
                .count();
            assert_eq!(sets, 1, "param {hash}");
            assert_eq!(last_value(&out, hash), latest.get(&hash).copied());
        }
        assert!(sim
            .edits
            .slots
            .values()
            .all(|s| s.flags.load(Ordering::Relaxed) == 0));
    }

    #[test]
    fn gesture_begun_live_and_ended_stalled_stays_balanced() {
        let mut sim = Sim::new(2);
        let mut open = HashMap::new();
        sim.edit(0, Edit::Begin, false);
        sim.edit(0, Edit::Set(0.1), false);
        // The host stops processing mid-drag, without draining
        sim.edit(0, Edit::Set(0.2), true);
        sim.edit(0, Edit::End, true);
        sim.edit(0, Edit::Begin, true);
        sim.edit(0, Edit::Set(0.3), true);
        // A live edit after the stall ends must join the pending bundle, not overtake it
        sim.edit(0, Edit::Set(0.4), false);
        let out = sim.drain();
        assert_balanced(&out, &mut open);
        assert_eq!(last_value(&out, 0), Some(0.4));
        assert_eq!(open.get(&0), Some(&true));

        sim.edit(0, Edit::End, false);
        let out = sim.drain();
        assert_eq!(out, [Event::End(0)]);
        assert_balanced(&out, &mut open);
    }

    #[test]
    fn stalled_gesture_ended_live_stays_balanced() {
        let mut sim = Sim::new(1);
        let mut open = HashMap::new();
        sim.edit(0, Edit::Begin, true);
        sim.edit(0, Edit::Set(0.6), true);
        let out = sim.drain();
        assert_eq!(out, [Event::Begin(0), Event::Set(0, 0.6)]);
        assert_balanced(&out, &mut open);

        sim.edit(0, Edit::Set(0.7), false);
        sim.edit(0, Edit::End, false);
        let out = sim.drain();
        assert_eq!(out, [Event::Set(0, 0.7), Event::End(0)]);
        assert_balanced(&out, &mut open);
    }

    #[test]
    fn host_set_after_gui_write_wins() {
        let mut sim = Sim::new(1);
        sim.edit(0, Edit::Begin, true);
        sim.edit(0, Edit::Set(0.6), true);
        sim.edit(0, Edit::End, true);
        sim.edits.host_set(0);
        assert_eq!(sim.drain(), [Event::Begin(0), Event::End(0)]);
    }

    #[test]
    fn state_load_drops_stalled_values_but_keeps_gestures() {
        let mut sim = Sim::new(2);
        let mut open = HashMap::new();
        sim.edit(0, Edit::Begin, true);
        sim.edit(0, Edit::Set(0.6), true);
        sim.edit(0, Edit::End, true);
        sim.edit(1, Edit::Begin, true);
        sim.edit(1, Edit::Set(0.3), true);
        sim.edits.drop_values();
        let out = sim.drain();
        assert_eq!(out, [Event::Begin(0), Event::End(0), Event::Begin(1)]);
        assert_balanced(&out, &mut open);

        sim.edit(1, Edit::End, false);
        let out = sim.drain();
        assert_eq!(out, [Event::End(1)]);
        assert_balanced(&out, &mut open);
    }

    #[test]
    fn dropped_marker_is_queued_again_by_the_next_edit() {
        let mut sim = Sim::new(1);
        sim.edit(0, Edit::Begin, true);
        sim.queue.clear();
        sim.edits.marker_dropped(0);
        sim.edit(0, Edit::Set(0.9), false);
        assert_eq!(sim.queue, [Event::Marker(0)]);
        assert_eq!(sim.drain(), [Event::Begin(0), Event::Set(0, 0.9)]);
    }
}
