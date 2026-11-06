//! Batched atomic accounting, isolated from lifecycle metadata and flushed on unwind.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[repr(align(64))]
#[derive(Default)]
pub(crate) struct Counters {
    total: AtomicU64,
}
impl Counters {
    #[cfg(test)]
    fn new(base: u64) -> Self {
        Self {
            total: AtomicU64::new(base),
        }
    }
    pub(crate) fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
    pub(crate) fn writers(self: &Arc<Self>, count: usize) -> Vec<Writer> {
        (0..count)
            .map(|_| Writer {
                bank: self.clone(),
                pending: 0,
            })
            .collect()
    }
}

// Not Clone: each pending batch is accounted exactly once, including on panic.
pub(crate) struct Writer {
    bank: Arc<Counters>,
    pending: u64,
}
impl Writer {
    pub(crate) fn record(&mut self) {
        self.pending += 1;
    }
    pub(crate) fn flush(&mut self) {
        if self.pending != 0 {
            self.bank.total.fetch_add(self.pending, Ordering::Relaxed);
            self.pending = 0;
        }
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.flush();
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
        assert!((Arc::as_ptr(&bank) as usize).is_multiple_of(64));
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
