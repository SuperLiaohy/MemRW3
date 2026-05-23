use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

/// Two-phase handshake for synchronizing main thread ↔ worker thread.
///
/// Matches the 3-semaphore pattern from MemRW2 (request → response → resume):
/// - `send_request(f)`: main thread blocks until worker pauses, executes `f`,
///   then signals worker to resume. `f` runs on the **main thread** with
///   exclusive access to shared resources (e.g., probe session).
/// - `try_acquire()`: worker thread non-blockingly checks for pending requests.
///   If a request is pending, acknowledges, blocks until main thread finishes,
///   then returns true.
///
/// # Lock-Free Data Path
/// During normal acquisition (running=true), the worker never calls `try_acquire`
/// and the main thread never calls `send_request` — both operate lock-free
/// on their respective halves of the DoubleBuffer.
pub struct Sync {
    request_pending: AtomicBool,
    mutex: Mutex<bool>,
    cv: Condvar,
}

impl Sync {
    pub fn new() -> Self {
        Self {
            request_pending: AtomicBool::new(false),
            mutex: Mutex::new(false),
            cv: Condvar::new(),
        }
    }

    /// Called from the **main thread**. Blocks until the worker acknowledges
    /// the request and pauses. Executes `f` while the worker is safely paused.
    /// Then signals the worker to resume.
    pub fn send_request<F: FnOnce()>(&self, f: F) {
        self.request_pending.store(true, Ordering::Release);
        let mut paused = self.mutex.lock().unwrap();
        while !*paused {
            paused = self.cv.wait(paused).unwrap();
        }
        *paused = false;
        drop(paused);
        f();
        self.request_pending.store(false, Ordering::Release);
        self.cv.notify_one();
    }

    /// Called from the **worker thread**. Non-blocking.
    /// Returns true if a request was pending and has been processed.
    pub fn try_acquire(&self) -> bool {
        if self.request_pending.load(Ordering::Acquire) {
            let mut paused = self.mutex.lock().unwrap();
            *paused = true;
            self.cv.notify_one();
            while self.request_pending.load(Ordering::Acquire) {
                paused = self.cv.wait(paused).unwrap();
            }
            drop(paused);
            true
        } else {
            false
        }
    }
}
