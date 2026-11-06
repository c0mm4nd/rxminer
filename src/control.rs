use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone)]
pub struct Control {
    pub stop: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub reload: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}
impl Control {
    pub fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            stop,
            paused: Arc::new(AtomicBool::new(false)),
            reload: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        }
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    pub fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        let mut error = self.error.lock().unwrap_or_else(|error| error.into_inner());
        if error.is_none() {
            *error = Some(message);
        }
        self.stop.store(true, Ordering::Release);
    }
    pub fn error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn watch_thread(&self, name: &'static str) -> PanicGuard<'_> {
        PanicGuard {
            control: self,
            name,
        }
    }
}

// Notify the coordinator during unwinding, without waiting for a future join.
// The failed worker is never resumed; its VM and other owned resources drop.
pub(crate) struct PanicGuard<'a> {
    control: &'a Control,
    name: &'static str,
}
impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.control.fail(format!("{} panicked", self.name));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Barrier, thread};

    #[test]
    fn concurrent_failures_publish_one_stable_error() {
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let barrier = Barrier::new(17);
        thread::scope(|scope| {
            for index in 0..16 {
                let control = &control;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    control.fail(format!("failure {index}"));
                });
            }
            barrier.wait();
            while !control.stopped() {
                thread::yield_now();
            }
            assert!(control.error().is_some());
        });
        let first = control.error();
        control.fail("later error");
        assert_eq!(control.error(), first);
    }

    #[test]
    fn panic_notifies_coordinator_before_join() {
        let control = Control::new(Arc::new(AtomicBool::new(false)));
        let worker_control = control.clone();
        let worker = thread::spawn(move || {
            let _guard = worker_control.watch_thread("test worker");
            panic!("injected worker panic");
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !control.stopped() && std::time::Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(control.stopped());
        assert_eq!(control.error().as_deref(), Some("test worker panicked"));
        assert!(worker.join().is_err());
    }
}
