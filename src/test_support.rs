//! Test-only helpers shared across this crate's `#[cfg(test)] mod tests`
//! blocks. Kept in its own file so a single `use crate::test_support::*;`
//! from any module gets the whole thing — no per-module prelude duplication.
//!
//! Two contracts live here and nowhere else:
//!
//! 1. **One crate-wide env lock.** Edition 2024 made `std::env::set_var` and
//!    `std::env::remove_var` unsafe because they are unsound while other
//!    threads read `environ`. Rust runs a test binary's tests in parallel, so
//!    *every* test that mutates the environment — not only the one module
//!    that happens to own a lock — must serialize through [`env_lock`].
//! 2. **Restoring guards, not manual unset.** [`setenv`]/[`unsetenv`] return
//!    an [`EnvGuard`] that puts the previous value back on drop. A test whose
//!    assertion panics mid-way therefore cannot leak a process-global
//!    variable (`WOX_DIRECTORY_USER_DATA` especially) into the tests that
//!    run after it.

/// Take the crate-wide environment lock. Hold it for the whole body of any
/// test that mutates env vars; a poisoned lock is reused so one panicking
/// test does not cascade into the others.
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Restore one variable to its pre-test state when dropped.
pub(crate) struct EnvGuard {
    key: String,
    prev: Option<std::ffi::OsString>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: the caller holds the crate-wide env lock for the guard's
        // whole lifetime (the guard is dropped before the lock, per binding
        // order in every call site).
        unsafe {
            match self.prev.take() {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

/// Set an environment variable, returning a guard that restores the previous
/// value (or unsets it) on drop.
///
/// **Caller is responsible for thread safety**: take [`env_lock`] first and
/// keep it for the whole test body.
pub(crate) unsafe fn setenv(
    key: &str,
    value: impl AsRef<std::ffi::OsStr>,
) -> EnvGuard {
    let prev = std::env::var_os(key);
    // SAFETY: the caller takes the crate-wide test env lock.
    unsafe {
        std::env::set_var(key, value);
    }
    EnvGuard {
        key: key.to_string(),
        prev,
    }
}

/// Remove an environment variable, returning a guard that restores its
/// previous value on drop. See [`setenv`] for the soundness contract.
pub(crate) unsafe fn unsetenv(key: &str) -> EnvGuard {
    let prev = std::env::var_os(key);
    // SAFETY: the caller takes the crate-wide test env lock.
    unsafe {
        std::env::remove_var(key);
    }
    EnvGuard {
        key: key.to_string(),
        prev,
    }
}
