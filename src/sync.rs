use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

pub struct Sync {
    request_pending: AtomicBool,
    cv_mutex: Mutex<bool>,
    cv_main: Condvar,
    done_mutex: Mutex<bool>,
    cv_worker: Condvar,
}

impl Sync {
    pub fn new() -> Self {
        Self {
            request_pending: AtomicBool::new(false),
            cv_mutex: Mutex::new(false),
            cv_main: Condvar::new(),
            done_mutex: Mutex::new(false),
            cv_worker: Condvar::new(),
        }
    }

    pub fn send_request<F: FnOnce()>(&self, f: F) {
        self.request_pending.store(true, Ordering::Release);
        let mut paused = self.cv_mutex.lock().unwrap();
        while !*paused {
            paused = self.cv_main.wait(paused).unwrap();
        }
        *paused = false;
        drop(paused);
        f();
        {
            let mut done = self.done_mutex.lock().unwrap();
            *done = true;
        }
        self.request_pending.store(false, Ordering::Release);
        self.cv_worker.notify_one();
    }

    pub fn try_acquire(&self) -> bool {
        if self.request_pending.load(Ordering::Acquire) {
            {
                let mut paused = self.cv_mutex.lock().unwrap();
                *paused = true;
            }
            self.cv_main.notify_one();
            {
                let mut done = self.done_mutex.lock().unwrap();
                while !*done {
                    done = self.cv_worker.wait(done).unwrap();
                }
                *done = false;
            }
            true
        } else {
            false
        }
    }
}
