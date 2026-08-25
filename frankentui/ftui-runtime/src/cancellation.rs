// SPDX-License-Identifier: Apache-2.0
//! Cooperative cancellation tokens for commands and tasks.
//!
//! [`CancellationToken`] provides a thread-safe, cloneable signal that tasks
//! can poll to detect cooperative cancellation requests. It extends the
//! subscription [`StopSignal`](crate::StopSignal) pattern to the command/task
//! domain, enabling bounded-lifetime effects and graceful teardown.
//!
//! # Migration rationale
//!
//! Web frameworks use `AbortController` / `AbortSignal` for effect cancellation.
//! This module provides an equivalent Rust-native primitive that the migration
//! code emitter can target when translating cancellable async workflows.
//!
//! # Example
//!
//! ```
//! use ftui_runtime::cancellation::{CancellationSource, CancellationToken};
//! use std::time::Duration;
//!
//! let source = CancellationSource::new();
//! let token = source.token();
//!
//! // Pass token to a background task
//! std::thread::spawn(move || {
//!     while !token.is_cancelled() {
//!         // do work...
//!         std::thread::sleep(Duration::from_millis(10));
//!     }
//! });
//!
//! // Cancel from the control side
//! source.cancel();
//! ```

#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use web_time::Duration;

/// A thread-safe, cloneable cancellation token.
///
/// Tasks and effects receive a token and poll [`is_cancelled`](Self::is_cancelled)
/// to detect cancellation requests. Tokens are cheap to clone and share across
/// thread boundaries.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

/// The control handle that triggers cancellation.
///
/// Dropping the source does **not** cancel the token — call [`cancel`](Self::cancel)
/// explicitly. This prevents accidental cancellation on scope exit.
pub struct CancellationSource {
    inner: Arc<CancellationInner>,
}

struct CancellationInner {
    cancelled: AtomicBool,
    notify: (Mutex<()>, Condvar),
}

impl CancellationSource {
    /// Create a new cancellation source with an uncancelled token.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                notify: (Mutex::new(()), Condvar::new()),
            }),
        }
    }

    /// Obtain a cloneable token that observes this source's state.
    pub fn token(&self) -> CancellationToken {
        CancellationToken {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Signal cancellation. All tokens derived from this source will observe
    /// `is_cancelled() == true` and any pending `wait_timeout` calls will wake.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        let (lock, cvar) = &self.inner.notify;
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        cvar.notify_all();
    }

    /// Check whether cancellation has already been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    /// Returns `true` if cancellation has been requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// Block until either cancellation is requested or the timeout elapses.
    ///
    /// Returns `true` if cancelled, `false` if timed out.
    pub fn wait_timeout(&self, duration: Duration) -> bool {
        if self.is_cancelled() {
            return true;
        }
        let (lock, cvar) = &self.inner.notify;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let start = web_time::Instant::now();
        let mut remaining = duration;
        loop {
            if self.is_cancelled() {
                return true;
            }
            let (new_guard, result) = cvar
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|e| e.into_inner());
            guard = new_guard;
            if self.is_cancelled() {
                return true;
            }
            if result.timed_out() {
                return false;
            }
            let elapsed = start.elapsed();
            if elapsed >= duration {
                return false;
            }
            remaining = duration - elapsed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering as AO;
    use std::thread;

    #[test]
    fn token_starts_uncancelled() {
        let source = CancellationSource::new();
        let token = source.token();
        assert!(!token.is_cancelled());
        assert!(!source.is_cancelled());
    }

    #[test]
    fn cancel_propagates_to_token() {
        let source = CancellationSource::new();
        let token = source.token();
        source.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_propagates_to_all_clones() {
        let source = CancellationSource::new();
        let t1 = source.token();
        let t2 = t1.clone();
        let t3 = source.token();
        source.cancel();
        assert!(t1.is_cancelled());
        assert!(t2.is_cancelled());
        assert!(t3.is_cancelled());
    }

    #[test]
    fn drop_source_does_not_cancel() {
        let source = CancellationSource::new();
        let token = source.token();
        drop(source);
        assert!(!token.is_cancelled());
    }

    #[test]
    fn wait_timeout_returns_true_when_already_cancelled() {
        let source = CancellationSource::new();
        let token = source.token();
        source.cancel();
        assert!(token.wait_timeout(Duration::from_secs(10)));
    }

    #[test]
    fn wait_timeout_returns_false_on_timeout() {
        let source = CancellationSource::new();
        let token = source.token();
        assert!(!token.wait_timeout(Duration::from_millis(10)));
    }

    #[test]
    fn wait_timeout_wakes_on_cancel() {
        let source = CancellationSource::new();
        let token = source.token();

        let handle = thread::spawn(move || token.wait_timeout(Duration::from_secs(10)));

        thread::sleep(Duration::from_millis(20));
        source.cancel();

        let result = handle.join().unwrap();
        assert!(result);
    }

    #[test]
    fn cancel_is_idempotent() {
        let source = CancellationSource::new();
        let token = source.token();
        source.cancel();
        source.cancel();
        source.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn token_works_across_threads() {
        let source = CancellationSource::new();
        let token = source.token();
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();

        let handle = thread::spawn(move || {
            while !token.is_cancelled() {
                thread::sleep(Duration::from_millis(5));
            }
            flag_clone.store(true, AO::SeqCst);
        });

        thread::sleep(Duration::from_millis(20));
        source.cancel();
        handle.join().unwrap();
        assert!(flag.load(AO::SeqCst));
    }

    #[test]
    fn default_creates_uncancelled_source() {
        let source = CancellationSource::default();
        assert!(!source.is_cancelled());
    }
}
