use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lock-free double buffer for single-producer single-consumer.
///
/// # Safety
/// - `push()` must be called only from the producer (acq_thread)
/// - `drain()` must be called only from the consumer (main thread)
/// - `latest()` may be called from the consumer
/// - Buffers are pre-allocated to avoid frequent reallocation
pub struct DoubleBuffer<T> {
    bufs: [UnsafeCell<Vec<T>>; 2],
    write_idx: AtomicUsize,
}

// SAFETY: DoubleBuffer is Sync because the producer only accesses
// bufs[write_idx] mutably via push(), and the consumer atomically
// swaps write_idx and takes exclusive ownership of the old buffer
// via drain(). The two threads never access the same buffer index
// concurrently.
unsafe impl<T> Sync for DoubleBuffer<T> {}

impl<T> DoubleBuffer<T> {
    pub fn new() -> Self {
        DoubleBuffer::with_capacity(2560)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            bufs: [
                UnsafeCell::new(Vec::with_capacity(cap)),
                UnsafeCell::new(Vec::with_capacity(cap)),
            ],
            write_idx: AtomicUsize::new(0),
        }
    }

    /// Push an item to the active write buffer. Called from the producer.
    pub fn push(&self, item: T) {
        let idx = self.write_idx.load(Ordering::Acquire);
        // SAFETY: Only the producer calls push(), and it only writes to
        // the buffer at write_idx. The consumer never accesses this index
        // after drain() atomically swaps it away.
        unsafe { &mut *self.bufs[idx].get() }.push(item);
    }

    /// Atomically swap buffers and return all items from the old buffer.
    /// Called from the consumer. The returned Vec is exclusively owned.
    pub fn drain(&self) -> Vec<T> {
        // fetch_xor(1) atomically flips the index between 0 and 1.
        // old_idx was the write buffer, now it becomes the read buffer
        // owned exclusively by the caller.
        let old_idx = self.write_idx.fetch_xor(1, Ordering::AcqRel);
        // SAFETY: After fetch_xor, the producer now writes to the other
        // buffer. old_idx is exclusively owned by the consumer.
        let buf = unsafe { &mut *self.bufs[old_idx].get() };
        let cap = buf.capacity().max(64);
        std::mem::replace(buf, Vec::with_capacity(cap))
    }

    /// Get the most recently pushed item from the active write buffer.
    /// Called from the consumer for snapshot reads (e.g., Table view).
    pub fn latest(&self) -> Option<T>
    where
        T: Clone,
    {
        let idx = self.write_idx.load(Ordering::Acquire);
        // SAFETY: Only reading the last element of the active write buffer.
        // This is a snapshot — the producer may concurrently push more items,
        // but we only read what's already there.
        let buf = unsafe { &*self.bufs[idx].get() };
        buf.last().cloned()
    }
}
