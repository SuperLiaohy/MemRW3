use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

pub struct Sync {
    request_mutex: Mutex<()>,
    request_pending: AtomicBool,
    cv_mutex: Mutex<bool>,
    cv_main: Condvar,
    done_mutex: Mutex<bool>,
    cv_worker: Condvar,
}

impl Sync {
    pub fn new() -> Self {
        Self {
            request_mutex: Mutex::new(()),
            request_pending: AtomicBool::new(false),
            cv_mutex: Mutex::new(false),
            cv_main: Condvar::new(),
            done_mutex: Mutex::new(false),
            cv_worker: Condvar::new(),
        }
    }

    pub fn send_request<R>(&self, f: impl FnOnce() -> R) -> R {
        let _request_guard = self
            .request_mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.request_pending.store(true, Ordering::Release);
        let mut paused = self
            .cv_mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*paused {
            paused = self
                .cv_main
                .wait(paused)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *paused = false;
        drop(paused);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        {
            let mut done = self
                .done_mutex
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *done = true;
        }
        self.request_pending.store(false, Ordering::Release);
        self.cv_worker.notify_one();
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub fn try_acquire(&self) -> bool {
        if self.request_pending.load(Ordering::Acquire) {
            {
                let mut paused = self
                    .cv_mutex
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *paused = true;
            }
            self.cv_main.notify_one();
            {
                let mut done = self
                    .done_mutex
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while !*done {
                    done = self
                        .cv_worker
                        .wait(done)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                *done = false;
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use super::Sync;

    fn worker(sync: Arc<Sync>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                sync.try_acquire();
                thread::yield_now();
            }
        })
    }

    #[test]
    fn returns_request_results_and_handles_sequential_requests() {
        let sync = Arc::new(Sync::new());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = worker(Arc::clone(&sync), Arc::clone(&stop));

        assert_eq!(sync.send_request(|| 21 * 2), 42);
        assert_eq!(sync.send_request(|| "done"), "done");

        stop.store(true, Ordering::Release);
        worker.join().unwrap();
    }

    #[test]
    fn resumes_worker_when_request_panics() {
        let sync = Arc::new(Sync::new());
        let stop = Arc::new(AtomicBool::new(false));
        let worker = worker(Arc::clone(&sync), Arc::clone(&stop));

        let panic = std::panic::catch_unwind({
            let sync = Arc::clone(&sync);
            move || sync.send_request(|| panic!("request failed"))
        });
        assert!(panic.is_err());
        assert_eq!(sync.send_request(|| 7), 7);

        stop.store(true, Ordering::Release);
        worker.join().unwrap();
    }
}
