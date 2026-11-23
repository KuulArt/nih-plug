//! Tells a host that stopped calling `process()` from one that is processing audio. REAPER's FX
//! bypass stops the audio callbacks without deactivating the plugin, so `is_processing` alone
//! would leave GUI parameter changes waiting for a `process()` call that never comes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long the host may go without calling `process()` before the audio counts as stalled.
pub const STALL: Duration = Duration::from_millis(200);

/// The time of the last `process()` call, stamped by the audio thread.
pub struct Heartbeat {
    epoch: Instant,
    /// Nanoseconds since `epoch` at the start of the last `process()` call.
    last_ns: AtomicU64,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            last_ns: AtomicU64::new(0),
        }
    }
}

impl Heartbeat {
    /// Called at the start of every `process()` call.
    pub fn beat(&self) {
        self.beat_at(self.now_ns());
    }

    /// Whether parameter changes can't count on a `process()` call to apply them: the plugin is
    /// not processing, or the host hasn't called `process()` for [`STALL`].
    pub fn audio_stalled(&self, is_processing: bool) -> bool {
        self.audio_stalled_at(is_processing, self.now_ns())
    }

    fn beat_at(&self, now_ns: u64) {
        self.last_ns.store(now_ns, Ordering::Relaxed);
    }

    fn audio_stalled_at(&self, is_processing: bool, now_ns: u64) -> bool {
        !is_processing
            || now_ns.saturating_sub(self.last_ns.load(Ordering::Relaxed))
                >= STALL.as_nanos() as u64
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn not_processing_is_stalled() {
        let hb = Heartbeat::default();
        hb.beat_at(1000 * MS);
        assert!(hb.audio_stalled_at(false, 1000 * MS));
    }

    #[test]
    fn fresh_heartbeat_is_live() {
        let hb = Heartbeat::default();
        hb.beat_at(1000 * MS);
        assert!(!hb.audio_stalled_at(true, 1000 * MS));
        assert!(!hb.audio_stalled_at(true, 1199 * MS));
    }

    #[test]
    fn heartbeat_older_than_the_stall_is_stalled() {
        let hb = Heartbeat::default();
        hb.beat_at(1000 * MS);
        assert!(hb.audio_stalled_at(true, 1200 * MS));
        assert!(hb.audio_stalled_at(true, 60_000 * MS));
        hb.beat_at(60_000 * MS);
        assert!(!hb.audio_stalled_at(true, 60_001 * MS));
    }
}
