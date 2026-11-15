//! Minimal fixed-size thread pool for independent work items.
//!
//! Governor fan-out is I/O bound (directory listings, small file probes), so
//! a small std-only pool is enough: no `rayon`, no channels, no extra crate —
//! the size-first release profile (ADR-008) stays intact.
//!
//! [`parallel_map`] guarantees two things discovery relies on:
//! 1. results come back in input order, never in completion order, and
//! 2. every item is processed exactly once, no matter how many workers run.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Upper bound on worker threads for a single batch.
///
/// Roots are a short list and each worker mostly blocks on the OS, so more
/// threads than that only adds scheduling overhead and open file handles.
pub const MAX_THREADS: usize = 8;

/// Worker count to use for a batch of `items`, at least 1 and at most
/// [`MAX_THREADS`]. Never larger than the number of items.
pub fn default_pool_size(items: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(MAX_THREADS)
        .max(1);
    items.min(cpus).max(1)
}

/// Apply `f` to every item using up to `threads` pooled worker threads.
///
/// The output is ordered like the input. Workers pull indices from a shared
/// atomic cursor, so a slow root cannot block the others (no chunking).
/// A panic inside `f` is re-raised in the caller when the batch ends.
pub fn parallel_map<T, R, F>(items: &[T], threads: usize, f: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send,
    F: Fn(&T) -> R + Send + Sync,
{
    let total = items.len();
    if total == 0 {
        return Vec::new();
    }

    let workers = threads.clamp(1, total);
    if workers == 1 {
        return items.iter().map(f).collect();
    }

    let cursor = AtomicUsize::new(0);
    // Each worker records (input index, result); sorting by index at the end
    // is what makes the output independent of thread scheduling.
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(total));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let value = f(&items[index]);
                let mut guard = done.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.push((index, value));
            });
        }
    });

    let mut pairs = done
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pairs.sort_by_key(|(index, _)| *index);
    debug_assert_eq!(pairs.len(), total, "each item must be handled once");
    pairs.into_iter().map(|(_, value)| value).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicIsize;
    use std::time::Duration;

    #[test]
    fn empty_input_yields_empty_output() {
        let items: Vec<u32> = Vec::new();
        assert!(parallel_map(&items, 4, |v| v * 2).is_empty());
    }

    #[test]
    fn output_keeps_input_order_not_completion_order() {
        // Later items finish first: the pool must still reorder them.
        let items: Vec<u64> = (0..12).collect();
        let out = parallel_map(&items, 4, |i| {
            if *i < 6 {
                std::thread::sleep(Duration::from_millis(2));
            }
            i * 10
        });
        assert_eq!(out, (0..12u64).map(|i| i * 10).collect::<Vec<_>>());
    }

    #[test]
    fn every_item_is_processed_exactly_once() {
        let hits: Vec<AtomicUsize> = (0..40).map(|_| AtomicUsize::new(0)).collect();
        let items: Vec<usize> = (0..40).collect();
        let out = parallel_map(&items, 5, |i| {
            hits[*i].fetch_add(1, Ordering::SeqCst);
            *i
        });
        assert_eq!(out.len(), 40);
        assert_eq!(out, items);
        assert!(
            hits.iter().all(|c| c.load(Ordering::SeqCst) == 1),
            "some item was processed more than once"
        );
    }

    #[test]
    fn one_thread_is_the_sequential_path() {
        let items: Vec<i32> = (-10..10).collect();
        let parallel = parallel_map(&items, 1, |v| v * 3);
        let sequential: Vec<i32> = items.iter().map(|v| v * 3).collect();
        assert_eq!(parallel, sequential);
    }

    #[test]
    fn thread_count_is_clamped_to_items_and_at_least_one() {
        let items = vec![1u32, 2, 3];
        assert_eq!(parallel_map(&items, 100, |v| v * v), vec![1, 4, 9]);
        assert_eq!(parallel_map(&items, 0, |v| v * v), vec![1, 4, 9]);
    }

    #[test]
    fn workers_overlap_but_never_exceed_the_pool_size() {
        // Proof that the scan really is concurrent, and that it respects the
        // pool size: every item samples how many workers are in flight. The
        // sleep is short on purpose — the suite shares this machine with
        // timing-sensitive tests elsewhere.
        let in_flight = AtomicIsize::new(0);
        let peak = AtomicIsize::new(0);
        let items: Vec<u8> = vec![0; 4];
        parallel_map(&items, 2, |_| {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(10));
            in_flight.fetch_sub(1, Ordering::SeqCst);
        });
        let peak = peak.load(Ordering::SeqCst);
        assert_eq!(peak, 2, "workers did not overlap (peak {peak})");
    }

    #[test]
    fn default_pool_size_is_bounded_and_non_zero() {
        assert_eq!(default_pool_size(0), 1);
        assert_eq!(default_pool_size(1), 1);
        for items in [2usize, 8, 64, 10_000] {
            let size = default_pool_size(items);
            assert!(size >= 1 && size <= MAX_THREADS, "bad pool size {size}");
            assert!(size <= items);
        }
        assert!(default_pool_size(100_000) <= MAX_THREADS);
    }
}
