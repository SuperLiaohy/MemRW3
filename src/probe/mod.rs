mod session;
mod worker;
pub use session::*;
pub use worker::*;

#[cfg(test)]
mod tests {
    use super::ProbeCommand;

    fn assert_send<T: Send>() {}

    #[test]
    fn probe_commands_are_send_to_the_worker_thread() {
        assert_send::<ProbeCommand>();
    }
}
