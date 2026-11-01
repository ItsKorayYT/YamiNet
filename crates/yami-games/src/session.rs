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
    released: AtomicU32,
    pointer: Mutex<(i32, i32)>,
    reset: AtomicBool,
    notified: AtomicBool,
    cancelled: AtomicBool,
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
        Self::launch_with("Local game".into(), move |_| load_local(&path), wake)
    }
    /// Load bytes on an admitted game worker. The cancellation flag is set by stop/drop.
    /// The loader must bound blocking I/O independently and check cancellation between transfers.
    pub fn launch_with(
        title: String,
        load: impl FnOnce(&AtomicBool) -> Result<(Manifest, Vec<u8>), String> + Send + 'static,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        WORKERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_SESSIONS).then_some(n + 1)
            })
            .map_err(|_| "At most four game sessions may be open")?;
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot {
                title,
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
            released: AtomicU32::new(0),
            pointer: Mutex::new((-1, -1)),
            reset: AtomicBool::new(false),
            notified: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
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
                if let Err(error) = worker.run(load, &wake) {
                    worker.snapshot.lock().unwrap().status =
                        if worker.cancelled.load(Ordering::Acquire) {
                            Status::Stopped
                        } else {
                            Status::Failed(error.chars().take(500).collect())
                        };
                    worker.notify(&wake);
                } else {
                    worker.snapshot.lock().unwrap().status = Status::Stopped;
                    worker.notify(&wake);
                }
                // Terminal tabs must not accumulate decoded asset caches.
                worker.snapshot.lock().unwrap().frame = None;
                worker.notify(&wake);
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
        let bit = bit & INPUT_MASK;
        if down && self.shared.desired.load(Ordering::Acquire) == RUN {
            let old = self.shared.held.fetch_or(bit, Ordering::AcqRel);
            self.shared.pressed.fetch_or(bit & !old, Ordering::AcqRel);
        } else {
            let old = self.shared.held.fetch_and(!bit, Ordering::AcqRel);
            self.shared.released.fetch_or(old & bit, Ordering::AcqRel);
        }
    }
    pub fn pointer(&self, x: i32, y: i32) {
        *self.shared.pointer.lock().unwrap() = if self.shared.desired.load(Ordering::Acquire) == RUN
            && (0..WIDTH).contains(&x)
            && (0..HEIGHT).contains(&y)
        {
            (x, y)
        } else {
            (-1, -1)
        };
    }
    pub fn clear_input(&self) {
        let old = self.shared.held.swap(0, Ordering::AcqRel);
        self.shared.released.fetch_or(old, Ordering::AcqRel);
        *self.shared.pointer.lock().unwrap() = (-1, -1);
        self.shared.pressed.store(0, Ordering::Release);
    }
    fn control(&self, value: u8) {
        let _guard = self.shared.signal.lock().unwrap();
        // Once shutdown is requested, pause/resume/reset cannot cancel it.
        if self.shared.desired.load(Ordering::Acquire) >= STOP && value < QUIT {
            return;
        }
        if value >= STOP {
            self.shared.cancelled.store(true, Ordering::Release);
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
    fn run(
        &self,
        load: impl FnOnce(&AtomicBool) -> Result<(Manifest, Vec<u8>), String>,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), String> {
        let start = Instant::now();
        if self.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let loaded = load(&self.cancelled);
        if self.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let (manifest, bytes) = loaded?;
        if self.desired.load(Ordering::Acquire) >= STOP {
            return Ok(());
        }
        let assets = Arc::clone(&manifest.prepared);
        let mut runtime = Runtime::with_assets(&bytes, Some(manifest.abi), Arc::clone(&assets))?;
        let mut audio = crate::audio::Output::new(!assets.sounds.is_empty());
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
        if self.desired.load(Ordering::Acquire) >= STOP {
            return Ok(());
        }
        audio.pause(self.desired.load(Ordering::Acquire) == PAUSE);
        audio.apply(&runtime.take_audio(), &assets);
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
                audio.pause(true);
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
            audio.pause(false);
            if self.reset.swap(false, Ordering::AcqRel) {
                audio.clear();
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
            let (x, y) = *self.pointer.lock().unwrap();
            let frame = runtime.frame_input(Input {
                held: self.held.load(Ordering::Acquire),
                pressed: self.pressed.swap(0, Ordering::AcqRel),
                released: self.released.swap(0, Ordering::AcqRel),
                x,
                y,
            })?;
            // Only successful, still-running frames can trigger native output.
            if self.desired.load(Ordering::Acquire) == RUN {
                audio.apply(&runtime.take_audio(), &assets);
            }
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
        let (send, receive) = std::sync::mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let done = exited.clone();
        let loading = Session::launch_with(
            "Remote transfer".into(),
            move |cancel| {
                send.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                done.store(true, Ordering::Release);
                Err("cancelled".into())
            },
            Arc::new(|| {}),
        )
        .unwrap();
        receive.recv_timeout(Duration::from_secs(3)).unwrap();
        let shared = loading.shared.clone();
        drop(loading);
        wait(&shared, |s| s.status == Status::Stopped);
        assert!(exited.load(Ordering::Acquire));
        assert_eq!(shared.snapshot.lock().unwrap().frames, 0);
        let start = Instant::now();
        while WORKERS.load(Ordering::SeqCst) != 0 {
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::yield_now();
        }
        let game = Session::launch(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../games/lantern-drift/game.yami-game.json"),
            Arc::new(|| {}),
        )
        .unwrap();
        wait(&game.shared, |s| s.frames > 1);
        game.key(UP | MOUSE_LEFT, true);
        game.pointer(639, 479);
        assert_eq!(game.shared.held.load(Ordering::Acquire), UP | MOUSE_LEFT);
        assert_eq!(*game.shared.pointer.lock().unwrap(), (639, 479));
        game.key(UP, false);
        assert_eq!(game.shared.held.load(Ordering::Acquire), MOUSE_LEFT);
        game.pause();
        wait(&game.shared, |s| s.status == Status::Paused);
        assert_eq!(game.shared.held.load(Ordering::Acquire), 0);
        assert_eq!(game.shared.pressed.load(Ordering::Acquire), 0);
        assert_eq!(*game.shared.pointer.lock().unwrap(), (-1, -1));
        game.key(DOWN, true);
        game.pointer(50, 50);
        assert_eq!(game.shared.held.load(Ordering::Acquire), 0);
        assert_eq!(*game.shared.pointer.lock().unwrap(), (-1, -1));
        game.resume();
        game.key(MOUSE_RIGHT, true);
        game.stop();
        assert_eq!(game.shared.held.load(Ordering::Acquire), 0);
        wait(&game.shared, |s| {
            s.status == Status::Stopped && s.frame.is_none()
        });
    }
}
