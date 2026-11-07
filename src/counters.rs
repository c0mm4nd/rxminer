//! Worker-local accounting; readers aggregate isolated slots, including retired workers.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

// Also separate slots on Apple Silicon, whose cache lines can be 128 bytes.
#[repr(align(128))]
#[derive(Default)]
struct Slot {
    total: AtomicU64,
}

#[derive(Default)]
struct State {
    retired: u64,
    slots: Vec<Arc<Slot>>,
}

#[derive(Default)]
pub(crate) struct Counters {
    state: Mutex<State>,
}
impl Counters {
    #[cfg(test)]
    fn new(base: u64) -> Self {
        Self {
            state: Mutex::new(State {
                retired: base,
                slots: Vec::new(),
            }),
        }
    }
    pub(crate) fn total(&self) -> u64 {
        let state = self.state.lock().unwrap();
        state.retired
            + state
                .slots
                .iter()
                .map(|slot| slot.total.load(Ordering::Relaxed))
                .sum::<u64>()
    }
    pub(crate) fn writers(self: &Arc<Self>, count: usize) -> Vec<Writer> {
        let mut state = self.state.lock().unwrap();
        (0..count)
            .map(|_| {
                let slot = Arc::new(Slot::default());
                state.slots.push(slot.clone());
                Writer {
                    bank: self.clone(),
                    slot,
                    total: 0,
                }
            })
            .collect()
    }
}

// Not Clone: each pending batch is accounted exactly once, including on panic.
pub(crate) struct Writer {
    bank: Arc<Counters>,
    slot: Arc<Slot>,
    total: u64,
}
impl Writer {
    pub(crate) fn record(&mut self) {
        self.total += 1;
    }
    pub(crate) fn flush(&mut self) {
        // Single writer: no read-modify-write or shared cache line on the hot path.
        self.slot.total.store(self.total, Ordering::Relaxed);
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        // Retire under the same lock used by readers: neither lost counts nor
        // double counting as a worker exits, including during unwinding.
        let mut state = self.bank.state.lock().unwrap();
        let index = state
            .slots
            .iter()
            .position(|slot| Arc::ptr_eq(slot, &self.slot))
            .expect("writer owns a registered slot");
        state.slots.swap_remove(index);
        state.retired += self.total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::AtomicUsize, thread};

    #[test]
    fn concurrent_counts_are_monotonic_and_exact_across_restart() {
        let bank = Arc::new(Counters::new(123));
        let writers = bank.writers(16);
        let completed = AtomicUsize::new(0);
        thread::scope(|scope| {
            for mut writer in writers {
                let completed = &completed;
                scope.spawn(move || {
                    for index in 0..100_003 {
                        writer.record();
                        if index % 8 == 0 {
                            writer.flush();
                        }
                    }
                    drop(writer); // also flushes the partial final batch
                    completed.fetch_add(1, Ordering::Release);
                });
            }
            let mut previous = 123;
            while completed.load(Ordering::Acquire) < 16 {
                let current = bank.total();
                assert!(current >= previous);
                previous = current;
            }
        });
        assert_eq!(bank.total(), 123 + 16 * 100_003);
        let next = bank.clone();
        let mut writers = next.writers(1);
        let before = bank.total();
        let mut writer = writers.pop().unwrap();
        writer.record();
        drop(writer);
        assert_eq!(next.total(), before + 1);
        assert!(bank.state.lock().unwrap().slots.is_empty());
        let writers = bank.writers(2);
        for writer in &writers {
            assert!((Arc::as_ptr(&writer.slot) as usize).is_multiple_of(128));
        }
    }

    #[test]
    fn unwinding_flushes_already_completed_hashes() {
        let bank = Arc::new(Counters::default());
        let mut writers = bank.writers(1);
        let mut writer = writers.pop().unwrap();
        assert!(thread::spawn(move || {
            writer.record();
            panic!("injected submit callback panic");
        })
        .join()
        .is_err());
        assert_eq!(bank.total(), 1);
    }
}
