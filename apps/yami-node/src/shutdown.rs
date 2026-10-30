//! Shared stop signal with interruptible background waits.
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
#[derive(Clone, Default)]
pub struct Shutdown(Arc<Signal>);
#[derive(Default)]
struct Signal {
    stopped: AtomicBool,
    mutex: Mutex<()>,
    changed: Condvar,
}
impl Shutdown {
    pub fn stop(&self) {
        let _guard = self.0.mutex.lock().unwrap_or_else(|e| e.into_inner());
        self.0.stopped.store(true, Ordering::SeqCst);
        self.0.changed.notify_all();
    }
    pub fn is_stopped(&self) -> bool {
        self.0.stopped.load(Ordering::SeqCst)
    }
    pub fn wait(&self, timeout: Duration) {
        let guard = self.0.mutex.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .0
            .changed
            .wait_timeout_while(guard, timeout, |_| !self.is_stopped());
    }
}
