use crate::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

static WORKERS: AtomicUsize = AtomicUsize::new(0);
pub const MAX_SESSIONS: usize = 4;
const RUN: u8 = 0;
const PAUSE: u8 = 1;
const STOP: u8 = 2;
const QUIT: u8 = 3;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    Running,
    Paused,
    Stopped,
    Failed(String),
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub title: String,
    pub description: String,
    pub status: Status,
    pub frame: Option<Arc<Frame>>,
    pub frames: u64,
    pub total_update_us: u64,
    pub max_update_us: u64,
    pub late_frames: u64,
    pub load_us: u64,
}
struct Shared {
    snapshot: Mutex<Snapshot>,
    signal: Mutex<()>,
    changed: Condvar,
    desired: AtomicU8,
    held: AtomicU32,
    pressed: AtomicU32,
    reset: AtomicBool,
    notified: AtomicBool,
}
/// A latest-frame mailbox and coalesced input: no unbounded queues, no VM on the caller.
/// Drop requests shutdown without joining on the UI thread. Fuel bounds guest shutdown latency.
pub struct Session {
    shared: Arc<Shared>,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameSession").finish_non_exhaustive()
    }
}
impl Session {
    pub fn launch(path: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        WORKERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_SESSIONS).then_some(n + 1)
            })
            .map_err(|_| "At most four local game sessions may be open")?;
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot {
                title: "Local game".into(),
                description: String::new(),
                status: Status::Loading,
                frame: None,
                frames: 0,
                total_update_us: 0,
                max_update_us: 0,
                late_frames: 0,
                load_us: 0,
            }),
            signal: Mutex::new(()),
            changed: Condvar::new(),
            desired: AtomicU8::new(RUN),
            held: AtomicU32::new(0),
            pressed: AtomicU32::new(0),
            reset: AtomicBool::new(false),
            notified: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        if let Err(error) = thread::Builder::new()
            .name("yami-game".into())
            .spawn(move || {
                struct Count;
                impl Drop for Count {
                    fn drop(&mut self) {
                        WORKERS.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                let _count = Count;
                if let Err(error) = worker.run(path, &wake) {
                    worker.snapshot.lock().unwrap().status =
                        Status::Failed(error.chars().take(500).collect());
                    worker.notify(&wake);
                } else {
                    worker.snapshot.lock().unwrap().status = Status::Stopped;
                    worker.notify(&wake);
                }
            })
        {
            WORKERS.fetch_sub(1, Ordering::SeqCst);
            return Err(error.to_string());
        }
        Ok(Self { shared })
    }
    pub fn snapshot(&self) -> Snapshot {
        // Clear before reading: a subsequent publication schedules another wake,
        // while an earlier publication is included in this snapshot.
        self.shared.notified.store(false, Ordering::Release);
        self.shared.snapshot.lock().unwrap().clone()
    }
    pub fn key(&self, bit: u32, down: bool) {
        let bit = bit & 15;
        if down && self.shared.desired.load(Ordering::Acquire) == RUN {
            let old = self.shared.held.fetch_or(bit, Ordering::AcqRel);
            self.shared.pressed.fetch_or(bit & !old, Ordering::AcqRel);
        } else {
            self.shared.held.fetch_and(!bit, Ordering::AcqRel);
        }
    }
    pub fn clear_input(&self) {
        self.shared.held.store(0, Ordering::Release);
        self.shared.pressed.store(0, Ordering::Release);
    }
    fn control(&self, value: u8) {
        let _guard = self.shared.signal.lock().unwrap();
        // Once shutdown is requested, pause/resume/reset cannot cancel it.
        if self.shared.desired.load(Ordering::Acquire) >= STOP && value < QUIT {
            return;
        }
        self.shared.desired.store(value, Ordering::Release);
        self.clear_input();
        self.shared.changed.notify_one();
    }
    pub fn pause(&self) {
        self.control(PAUSE);
    }
    pub fn resume(&self) {
        self.control(RUN);
    }
    pub fn stop(&self) {
        self.control(STOP);
    }
    pub fn reset(&self) {
        self.shared.reset.store(true, Ordering::Release);
        self.control(RUN);
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.control(QUIT);
    }
}
impl Shared {
    fn notify(&self, wake: &Arc<dyn Fn() + Send + Sync>) {
        if !self.notified.swap(true, Ordering::AcqRel) {
            wake();
        }
    }
    fn run(&self, path: PathBuf, wake: &Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
        let start = Instant::now();
        let (manifest, bytes) = load_local(&path)?;
        if self.desired.load(Ordering::Acquire) >= STOP {
            return Ok(());
        }
        let mut runtime = Runtime::new(&bytes)?;
        {
            let mut s = self.snapshot.lock().unwrap();
            s.title = manifest.title;
            s.description = manifest.description;
            s.load_us = start.elapsed().as_micros() as u64;
        }
        // Publish a first frame even when focus was lost during loading, so a
        // paused tab has a visible game screen instead of an empty canvas.
        if self.desired.load(Ordering::Acquire) >= STOP {
            return Ok(());
        }
        let first_start = Instant::now();
        let first = runtime.frame(0, 0)?;
        {
            let mut s = self.snapshot.lock().unwrap();
            s.frame = Some(first);
            s.frames = 1;
            s.total_update_us = first_start.elapsed().as_micros() as u64;
            s.max_update_us = s.total_update_us;
        }
        self.notify(wake);
        let tick = Duration::from_nanos(1_000_000_000 / 60);
        let mut next = Instant::now();
        loop {
            let desired = self.desired.load(Ordering::Acquire);
            if desired >= STOP {
                self.snapshot.lock().unwrap().status = Status::Stopped;
                self.notify(wake);
                return Ok(());
            }
            if desired == PAUSE {
                let changed = {
                    let mut s = self.snapshot.lock().unwrap();
                    let changed = s.status != Status::Paused;
                    s.status = Status::Paused;
                    changed
                };
                if changed {
                    self.notify(wake);
                }
                let guard = self.signal.lock().unwrap();
                if self.desired.load(Ordering::Acquire) == PAUSE {
                    drop(self.changed.wait(guard).unwrap());
                }
                next = Instant::now();
                continue;
            }
            if self.reset.swap(false, Ordering::AcqRel) {
                runtime.reset()?;
            }
            let now = Instant::now();
            if now < next {
                let guard = self.signal.lock().unwrap();
                // Recheck under the signal lock to avoid losing a shutdown/pause notification.
                if self.desired.load(Ordering::Acquire) == RUN {
                    drop(
                        self.changed
                            .wait_timeout(guard, next.saturating_duration_since(Instant::now()))
                            .unwrap(),
                    );
                }
                continue;
            }
            let frame = runtime.frame(
                self.held.load(Ordering::Acquire),
                self.pressed.swap(0, Ordering::AcqRel),
            )?;
            let elapsed = now.elapsed();
            {
                let mut s = self.snapshot.lock().unwrap();
                s.status = Status::Running;
                s.frame = Some(frame);
                s.frames += 1;
                let us = elapsed.as_micros() as u64;
                s.total_update_us += us;
                s.max_update_us = s.max_update_us.max(us);
                if now.saturating_duration_since(next) >= tick || elapsed >= tick {
                    s.late_frames += 1;
                }
            }
            self.notify(wake);
            // No catch-up spiral: a late frame slows simulation instead of growing a queue.
            next += tick;
            if Instant::now() >= next {
                next = Instant::now() + tick;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(shared: &Shared, check: impl Fn(&Snapshot) -> bool) {
        let start = Instant::now();
        while !check(&shared.snapshot.lock().unwrap()) {
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn closed_games_release_workers_including_during_load_and_enforce_global_cap() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../games/breakout/game.yami-game.json");
        // These sessions remain active while testing the shared admission cap.
        let mut sessions = Vec::new();
        for _ in 0..MAX_SESSIONS {
            sessions.push(Session::launch(path.clone(), Arc::new(|| {})).unwrap());
        }
        assert!(Session::launch(path.clone(), Arc::new(|| {})).is_err());
        let shared: Vec<_> = sessions.iter().map(|s| Arc::clone(&s.shared)).collect();
        drop(sessions);
        for shared in &shared {
            wait(shared, |s| s.status == Status::Stopped);
        }
        let start = Instant::now();
        while WORKERS.load(Ordering::SeqCst) != 0 {
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::yield_now();
        }
        let game = Session::launch(path, Arc::new(|| {})).unwrap();
        wait(&game.shared, |s| s.frames > 1);
        let shared = Arc::clone(&game.shared);
        drop(game);
        wait(&shared, |s| s.status == Status::Stopped);
    }
}
