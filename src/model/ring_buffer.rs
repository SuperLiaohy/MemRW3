use crossbeam_queue::ArrayQueue;

const DEFAULT_CAPACITY: usize = 2560;

/// Bounded lock-free queue used between the acquisition and UI threads.
///
/// `ArrayQueue` stores values in a fixed-size ring, so producer and consumer
/// can make progress concurrently without reallocating or touching the same
/// `Vec`. When the UI falls behind, the oldest sample is discarded so charts
/// continue to receive the most recent data.
pub struct RingBuffer<T> {
    queue: ArrayQueue<T>,
}

impl<T> RingBuffer<T> {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "ring buffer capacity must be greater than zero"
        );
        Self {
            queue: ArrayQueue::new(capacity),
        }
    }

    /// Push a value without blocking.
    ///
    /// Returns `true` when an older value had to be discarded.
    pub fn push(&self, item: T) -> bool {
        self.queue.force_push(item).is_some()
    }

    /// Replace `output` with the values currently available in FIFO order.
    ///
    /// The allocation owned by `output` is retained between calls. The
    /// producer may continue to push while this method is running.
    pub fn drain_into(&self, output: &mut Vec<T>) -> usize {
        output.clear();
        let available = self.queue.len();
        output.reserve(available);
        for _ in 0..available {
            match self.queue.pop() {
                Some(item) => output.push(item),
                None => break,
            }
        }
        output.len()
    }

    /// Discard the values currently available without allocating.
    pub fn discard_all(&self) -> usize {
        let available = self.queue.len();
        let mut discarded = 0;
        for _ in 0..available {
            if self.queue.pop().is_none() {
                break;
            }
            discarded += 1;
        }
        discarded
    }
}

impl<T> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::RingBuffer;

    #[test]
    fn drains_values_in_fifo_order() {
        let buffer = RingBuffer::with_capacity(4);
        let mut output = vec![99];
        assert!(!buffer.push(1));
        assert!(!buffer.push(2));
        assert_eq!(buffer.drain_into(&mut output), 2);
        assert_eq!(output, vec![1, 2]);
        assert_eq!(buffer.drain_into(&mut output), 0);
        assert!(output.is_empty());
    }

    #[test]
    fn discards_oldest_value_when_full() {
        let buffer = RingBuffer::with_capacity(2);
        assert!(!buffer.push(1));
        assert!(!buffer.push(2));
        assert!(buffer.push(3));
        let mut output = Vec::new();
        buffer.drain_into(&mut output);
        assert_eq!(output, vec![2, 3]);
    }

    #[test]
    fn reuses_output_allocation() {
        let buffer = RingBuffer::with_capacity(4);
        let mut output = Vec::with_capacity(4);
        let allocation = output.as_ptr();

        buffer.push(1);
        buffer.push(2);
        buffer.drain_into(&mut output);
        assert_eq!(output.as_ptr(), allocation);

        buffer.push(3);
        buffer.drain_into(&mut output);
        assert_eq!(output.as_ptr(), allocation);
        assert_eq!(output, vec![3]);
    }

    #[test]
    fn discards_without_an_output_buffer() {
        let buffer = RingBuffer::with_capacity(4);
        buffer.push(1);
        buffer.push(2);
        assert_eq!(buffer.discard_all(), 2);
        assert_eq!(buffer.discard_all(), 0);
    }

    #[test]
    fn producer_and_consumer_can_run_concurrently() {
        const ITEM_COUNT: usize = 20_000;
        let buffer = Arc::new(RingBuffer::with_capacity(ITEM_COUNT));
        let producer_buffer = Arc::clone(&buffer);
        let producer = thread::spawn(move || {
            for value in 0..ITEM_COUNT {
                assert!(!producer_buffer.push(value));
            }
        });

        let mut received = Vec::with_capacity(ITEM_COUNT);
        let mut batch = Vec::new();
        while received.len() < ITEM_COUNT {
            buffer.drain_into(&mut batch);
            for item in batch.drain(..) {
                received.push(item);
            }
            thread::yield_now();
        }
        producer.join().unwrap();

        assert_eq!(received, (0..ITEM_COUNT).collect::<Vec<_>>());
    }
}
