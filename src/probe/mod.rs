mod session;
pub use session::*;

use std::sync::Mutex;

/// Thread-safe owner of the probe session.
///
/// The acquisition handshake keeps this lock uncontended in normal operation;
/// the mutex also makes accidental overlapping control requests memory-safe.
pub struct ProbeCell(Mutex<ProbeSession>);

impl ProbeCell {
    pub fn new(session: ProbeSession) -> Self {
        Self(Mutex::new(session))
    }

    pub fn with_mut<R>(&self, f: impl FnOnce(&mut ProbeSession) -> R) -> R {
        let mut session = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut session)
    }
}

#[cfg(test)]
mod tests {
    use super::ProbeSession;

    fn assert_send<T: Send>() {}

    #[test]
    fn probe_session_is_send_without_a_cached_core() {
        assert_send::<ProbeSession>();
    }
}
