mod session;
mod worker;
pub use session::*;
pub use worker::*;

#[cfg(test)]
mod tests {
    use super::ProbeSession;

    fn assert_send<T: Send>() {}

    #[test]
    fn probe_session_is_send_to_the_worker_thread() {
        assert_send::<ProbeSession>();
    }
}
