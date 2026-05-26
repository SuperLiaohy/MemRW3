mod session;
pub use session::*;

use std::cell::UnsafeCell;

/// Mutable reference to ProbeSession shared between threads.
///
/// # Safety
/// Access is serialized by the `Sync` handshake:
/// - acq_thread during normal acquisition
/// - main thread during `Sync::send_request` closures (acq_thread paused)
/// Never accessed concurrently — Mutex overhead is unnecessary.
pub struct ProbeCell(UnsafeCell<ProbeSession>);

unsafe impl Send for ProbeCell {}
unsafe impl Sync for ProbeCell {}

impl ProbeCell {
    pub fn new(session: ProbeSession) -> Self {
        Self(UnsafeCell::new(session))
    }

    pub unsafe fn get_mut(&self) -> &mut ProbeSession {
        unsafe { &mut *self.0.get() }
    }

    pub fn get(&self) -> &ProbeSession {
        unsafe { &*self.0.get() }
    }
}
