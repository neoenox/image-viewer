//! Lock helpers that survive a poisoned `Mutex`.
//!
//! A panic on one worker thread must not turn every later lock into a panic and take
//! the whole viewer down. The guarded state stays usable: workers never leave it half
//! updated across a point that can panic (decoding runs outside the lock).
use std::sync::{Condvar, Mutex, MutexGuard};

pub(crate) trait LockRecover<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}
impl<T> LockRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(crate) fn wait_recover<'a, T>(wake: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    wake.wait(guard).unwrap_or_else(|e| e.into_inner())
}

/// Runs decoding work so that a panic inside a codec becomes an `Err` instead of
/// killing the worker thread (which would leave its job marked in flight forever).
pub(crate) fn catch_panic<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("decoder panicked".to_owned()))
}
