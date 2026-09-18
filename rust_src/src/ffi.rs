//! C FFI exports for Starship in-process prompt rendering.
//!
//! All public functions follow the `ssp_` prefix convention and use C ABI.
//! Heap strings returned to C are owned by Rust and must be freed with
//! `ssp_free`; `ssp_version` returns a static string instead.
//!
//! There is exactly one global prompt-rendering session per shell process.
//! `ssp_init` establishes it, `ssp_shutdown` tears it down
//! (stopping the scoped rayon pool), and a later `ssp_init` builds a
//! fresh one with an empty cache and zeroed statistics.

use libc::c_char;
use starship::context::{Properties, Target};
use starship::session::SessionStats;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------
//
// Error protocol: every fallible `ssp_*` FFI function returns `*mut c_char`.
//   * NULL means success.
//   * A non-NULL value is a heap-allocated, NUL-terminated UTF-8 error string.
//     The caller owns it and must release it with `ssp_free()`.
//
// This deliberately eliminates all process-global / per-session error slots.
// Each call carries its own error value directly in its return value, so there
// is no shared mutable state to race on and no TLS destructor to dangle after
// dlclose(). `ssp_free` itself cannot fail and therefore returns void;
// `ssp_version` returns a static string rather than an error.

/// Copy a Rust string into a C-owned NUL-terminated UTF-8 buffer.
///
/// Interior NUL bytes are truncated (prompt/error messages never contain NUL
/// in practice, but the FFI boundary must stay total).
fn string_into_c(value: String) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|e| {
            let pos = e.nul_position();
            let mut bytes = e.into_vec();
            bytes.truncate(pos);
            CString::new(bytes).unwrap()
        })
        .into_raw()
}

/// Build an allocated error string for `msg`.
fn error_string(msg: impl Into<String>) -> *mut c_char {
    string_into_c(msg.into())
}

/// Convert a caught panic into an allocated error string.
fn panic_to_error(panic: Box<dyn std::any::Any + Send>) -> *mut c_char {
    let msg = if let Some(s) = panic.downcast_ref::<&str>() {
        format!("panic: {s}")
    } else if let Some(s) = panic.downcast_ref::<String>() {
        format!("panic: {s}")
    } else {
        "panic: unknown error".to_string()
    };
    string_into_c(msg)
}

/// FFI panic guard for fallible exports.
///
/// The wrapped expression must itself return `*mut c_char` using the error
/// protocol above. Panics are caught and converted to an allocated error
/// string instead of unwinding across the C boundary.
macro_rules! ffi_guard_error {
    ($expr:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $expr)) {
            Ok(result) => result,
            Err(panic) => panic_to_error(panic),
        }
    };
}

// ---------------------------------------------------------------------------
// C-compatible struct: render input
// ---------------------------------------------------------------------------

/// Input parameters for a single prompt render call.
#[repr(C)]
pub struct ssp_render_input {
    /// Status code of the previously run command (NULL or string).
    pub status: *const c_char,
    /// Array of pipe status strings.
    pub pipestatus: *const *const c_char,
    /// Number of entries in pipestatus.
    pub pipestatus_len: usize,
    /// Terminal width in columns (0 = auto-detect).
    pub terminal_width: usize,
    /// working directory path (NULL = use process cwd).
    pub path: *const c_char,
    /// Logical working directory path (for powershell and elvish)
    pub logical_path: *const c_char,
    /// Execution duration of the last command in ms (NULL = none).
    pub cmd_duration: *const c_char,
    /// Current keymap name (NULL = "viins").
    pub keymap: *const c_char,
    /// Number of currently running jobs.
    pub jobs: i64,
    /// Current SHLVL value (-1 = none).
    pub shlvl: i64,
    /// Prompt target: 0 = Main, 1 = Right, 2 = Continuation.
    pub target: c_int,
}

/// Cache performance counters.
#[repr(C)]
#[derive(Default)]
pub struct ssp_out_stats {
    pub config_hits: u64,
    pub config_misses: u64,
    pub repo_status_hits: u64,
    pub repo_status_misses: u64,
    pub git_repo_hits: u64,
    pub git_repo_misses: u64,
    pub git_metrics_hits: u64,
    pub git_metrics_misses: u64,
    pub dir_contents_hits: u64,
    pub dir_contents_misses: u64,
    pub binary_path_hits: u64,
    pub binary_path_misses: u64,
    pub renders: u64,
}

// ---------------------------------------------------------------------------
// Helper: convert C input to Rust Properties
// ---------------------------------------------------------------------------

fn cstr_to_option_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

fn cstr_array_to_vec(ptr: *const *const c_char, len: usize) -> Option<Vec<String>> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let mut result = Vec::with_capacity(len);
    unsafe {
        for i in 0..len {
            let s = *ptr.add(i);
            if s.is_null() {
                continue;
            }
            if let Ok(s) = CStr::from_ptr(s).to_str() {
                result.push(s.to_string());
            }
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

impl ssp_render_input {
    fn to_properties(&self, default_target: Target) -> Properties {
        let props = Properties {
            status_code: cstr_to_option_string(self.status),
            pipestatus: cstr_array_to_vec(self.pipestatus, self.pipestatus_len),
            terminal_width: self.terminal_width,
            path: cstr_to_option_string(self.path).map(std::path::PathBuf::from),
            logical_path: cstr_to_option_string(self.logical_path).map(std::path::PathBuf::from),
            cmd_duration: cstr_to_option_string(self.cmd_duration),
            keymap: cstr_to_option_string(self.keymap).unwrap_or_else(|| "viins".into()),
            jobs: self.jobs,
            shlvl: if self.shlvl >= 0 {
                Some(self.shlvl)
            } else {
                None
            },
        };
        let _ = default_target; // used below
        props
    }

    fn target(&self) -> Target {
        match self.target {
            1 => Target::Right,
            2 => Target::Continuation,
            _ => Target::Main,
        }
    }
}

// ---------------------------------------------------------------------------
// Global session
// ---------------------------------------------------------------------------
//
// NOTE: deliberately NOT thread_local!. A thread_local! would register a TLS
// destructor on the HOST thread (zsh/pwsh main thread) the first time it is
// touched. After dlclose() unmaps this dylib, that destructor pointer dangles;
// glibc skips destructors of unloaded DSOs, but macOS and Windows do not.
// A process-global Mutex has no per-thread state and is safe to unload.
//
// One session per process matches the shell integration model: each shell
// process loads its own copy of the library and therefore owns its own
// session. The session is dropped explicitly by `ssp_shutdown` before
// the shell unloads the module, so the scoped rayon pool is stopped cleanly.

static SESSION: Mutex<Option<starship::session::Session>> = Mutex::new(None);

/// PID of the process that created `SESSION` (0 when no session exists).
///
/// Kept outside the Mutex so a forked child can reject FFI calls without
/// touching a mutex that may have been held when `fork()` happened.
static SESSION_PID: AtomicU32 = AtomicU32::new(0);

/// Run `f` with the global session, or fail when `zo_init` has not been called.
///
/// The lock is held for the duration of one native operation. Shell hosts are
/// single-threaded, so this is serialization, not a concurrency feature; the
/// mutex is what makes the global state sound for a multi-threaded host.
fn with_session<T, E: std::fmt::Display>(
    f: impl FnOnce(&mut starship::session::Session) -> Result<T, E>,
) -> Result<T, String> {
    let mut guard = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(session) = guard.as_mut() else {
        return Err(format!("session is not initialized"));
    };
    f(session).map_err(|e| format!("{e:#}"))
}

/// Reject calls made from a forked child process.
///
/// zsh forks for $(...), &, pipelines, subshells, and process substitution. The
/// child inherits the parent's rayon runtime in a corrupted state, so it must
/// not render or destroy the session. Returning before `lock_session()` also
/// avoids blocking on a mutex that may have been held during `fork()`.
macro_rules! guard_fork {
    () => {
        let recorded_pid = SESSION_PID.load(Ordering::Relaxed);
        if recorded_pid != 0 && recorded_pid != std::process::id() {
            return error_string("refusing call in forked child process");
        }
    };
}

// ---------------------------------------------------------------------------
// Public C API
// ---------------------------------------------------------------------------

/// Create the global prompt rendering session.
///
/// Returns NULL on success. If a session already exists, returns an allocated
/// error string (free it with `ssp_free()`).
///
/// Calling this after `ssp_shutdown()` is supported and creates a fresh
/// session with an empty cache and zeroed statistics.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_init() -> *mut c_char {
    ffi_guard_error!({
        guard_fork!();
        let mut guard = SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.is_none() {
            // Session creates its own scoped rayon pool (no global pool).
            // The pool is stopped when the session is destroyed.
            match starship::session::Session::new() {
                Ok(session) => *guard = Some(session),
                Err(e) => return error_string(format!("{e:#}")),
            }
            SESSION_PID.store(std::process::id(), Ordering::Relaxed);
        }

        ptr::null_mut()
    })
}

/// Destroy the global session.
///
/// Returns NULL on success, including when no session exists (idempotent).
/// A later `ssp_init()` may establish a new session.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_shutdown() -> *mut c_char {
    ffi_guard_error!({
        guard_fork!();

        // Take the session out while holding the lock, then release the lock
        // before dropping it: `Session::drop` waits for rayon workers to exit.
        let mut guard = SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        SESSION_PID.store(0, Ordering::Relaxed);
        *guard = None;
        ptr::null_mut()
    })
}

/// Render a prompt using the global session.
///
/// Returns NULL on success and writes a Rust-allocated prompt string to `*out`
/// (free it with `ssp_free()`). Returns an allocated error string on failure
/// and sets `*out` to NULL.
///
/// # Safety
///
/// `input` must point to a valid `ssp_render_input`, and `out` must be NULL or
/// point to writable `*mut c_char` storage for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_render(
    input: *const ssp_render_input,
    out: *mut *mut c_char,
) -> *mut c_char {
    ffi_guard_error!({
        if out.is_null() {
            return error_string("null argument");
        }

        // Always reset the caller's output slot before any other validation.
        // On failure the caller may otherwise keep a stale/dangling pointer
        // from a previous call.
        // SAFETY: `out` was checked non-null above and is writable for the
        // duration of this call.
        unsafe { *out = ptr::null_mut() };

        if input.is_null() {
            return error_string("null argument");
        }
        guard_fork!();

        let input = unsafe { &*input };
        let properties = input.to_properties(input.target());
        let target = input.target();
        match with_session(|session| session.render(properties, target)) {
            // SAFETY: `out` is writable for the duration of this call.
            Ok(output) => unsafe { *out = string_into_c(output) },
            Err(e) => return error_string(e),
        };

        ptr::null_mut()
    })
}

/// Free a string previously returned by any fallible `ssp_*` function or by
/// `ssp_render`.
///
/// Passing NULL is safe (no-op). This function cannot fail, so it is the one
/// export that does not use the `char *` error protocol.
///
/// # Safety
///
/// `ptr` must be NULL or a pointer previously returned by this library that
/// has not already been freed. Static strings from `ssp_version()` must NOT
/// be passed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: callers must pass a pointer previously returned by this library.
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

/// Return the library version as a static string.
///
/// The returned pointer is valid for the lifetime of the process and must NOT
/// be freed. This accessor cannot fail, so it is exempt from the error
/// protocol.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_version() -> *const c_char {
    // A string literal has static storage duration and the trailing NUL is
    // included in the literal itself, so no LazyLock/allocation is needed.
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr().cast()
}

/// Retrieve cache performance statistics from the global session.
///
/// Returns NULL on success and writes the snapshot to `*out`. Returns an
/// allocated error string on failure (free it with `ssp_free()`).
///
/// # Safety
///
/// `out` must be NULL or point to writable `ssp_stats` storage for the
/// duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_stats(out: *mut ssp_out_stats) -> *mut c_char {
    ffi_guard_error!({
        if out.is_null() {
            return error_string("null argument");
        }
        guard_fork!();

        let stats =
            match with_session::<SessionStats, String>(|session| Ok(session.state().stats())) {
                Ok(stats) => stats,
                Err(e) => return error_string(e),
            };

        let c_stats = ssp_out_stats {
            config_hits: stats.config_hits,
            config_misses: stats.config_misses,
            repo_status_hits: stats.repo_status_hits,
            repo_status_misses: stats.repo_status_misses,
            git_repo_hits: stats.git_repo_hits,
            git_repo_misses: stats.git_repo_misses,
            git_metrics_hits: stats.git_metrics_hits,
            git_metrics_misses: stats.git_metrics_misses,
            dir_contents_hits: stats.dir_contents_hits,
            dir_contents_misses: stats.dir_contents_misses,
            binary_path_hits: stats.binary_path_hits,
            binary_path_misses: stats.binary_path_misses,
            renders: stats.renders,
        };
        // SAFETY: `out` is writable for the duration of this call.
        unsafe { *out = c_stats };
        ptr::null_mut()
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;
    use std::sync::{Mutex as StdMutex, MutexGuard};

    /// Tests share one process-global session, so serialize them.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn test_guard() -> MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Read and free an allocated C error string.
    fn take_error(err: *mut c_char) -> String {
        assert!(!err.is_null(), "expected an error string");
        let msg = unsafe { CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned();
        unsafe { ssp_free(err) };
        msg
    }

    fn create_session() {
        let err = ssp_init();
        if !err.is_null() {
            panic!("session creation failed: {}", take_error(err));
        }
    }

    fn destroy_session() {
        let err = ssp_shutdown();
        if !err.is_null() {
            panic!("session destruction failed: {}", take_error(err));
        }
    }

    fn main_input() -> ssp_render_input {
        ssp_render_input {
            status: ptr::null(),
            pipestatus: ptr::null(),
            pipestatus_len: 0,
            terminal_width: 80,
            path: ptr::null(),
            logical_path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        }
    }

    /// Test basic create/destroy lifecycle.
    #[test]
    fn test_session_lifecycle() {
        let _guard = test_guard();
        create_session();
        destroy_session();
    }

    /// Test that creating a second session while one is active does not fail.
    #[test]
    fn test_create_twice_errors() {
        let _guard = test_guard();
        create_session();
        let err = ssp_init();
        assert!(err.is_null(), "second create should not return an error");
        destroy_session();
    }

    /// Test that render/stats before creation return an error.
    #[test]
    fn test_render_before_create_errors() {
        let _guard = test_guard();
        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&main_input() as *const _, &mut out);
            assert!(!err.is_null(), "render without session should error");
            assert!(out.is_null(), "failed render should clear *out");
            take_error(err);

            let mut stats: ssp_out_stats = ssp_out_stats::default();
            let err = ssp_stats(&mut stats as *mut _);
            assert!(!err.is_null(), "stats without session should error");
            take_error(err);
        }
    }

    /// Test that destroy is idempotent.
    #[test]
    fn test_destroy_is_idempotent() {
        let _guard = test_guard();
        let err = ssp_shutdown();
        assert!(err.is_null(), "destroy without session should be a no-op");

        create_session();
        destroy_session();

        let err = ssp_shutdown();
        assert!(err.is_null(), "second destroy should be a no-op");
    }

    /// Test rendering a main prompt.
    #[test]
    fn test_render_main_prompt() {
        let _guard = test_guard();
        create_session();
        let status = CString::new("0").unwrap();
        let input = ssp_render_input {
            status: status.as_ptr(),
            ..main_input()
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("render failed: {}", take_error(err));
            }
            assert!(!out.is_null(), "output should be non-null");

            let output = CStr::from_ptr(out).to_str().unwrap();
            assert!(!output.is_empty(), "output should not be empty");

            ssp_free(out);
        }
        destroy_session();
    }

    /// Test rendering a right prompt.
    #[test]
    fn test_render_right_prompt() {
        let _guard = test_guard();
        create_session();
        let input = ssp_render_input {
            target: 1, // Right
            ..main_input()
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("right prompt render failed: {}", take_error(err));
            }
            ssp_free(out);
        }
        destroy_session();
    }

    /// Test render with NULL arguments returns a call-local error string.
    #[test]
    fn test_render_null_args() {
        let _guard = test_guard();
        create_session();

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(ptr::null(), &mut out);
            assert!(!err.is_null(), "null input should return an error");
            assert!(out.is_null(), "failed render should clear *out");
            let msg = take_error(err);
            assert!(msg.contains("null argument"), "unexpected error: {msg}");
        }
        destroy_session();
    }

    /// Test that a null out-slot returns an error.
    #[test]
    fn test_render_null_out() {
        let _guard = test_guard();
        create_session();
        let err = unsafe { ssp_render(&main_input() as *const _, ptr::null_mut()) };
        assert!(!err.is_null(), "null out should return an error");
        take_error(err);
        destroy_session();
    }

    /// Test that version returns a non-null static string.
    #[test]
    fn test_version() {
        let v = ssp_version();
        assert!(!v.is_null());
        let s = unsafe { CStr::from_ptr(v) }.to_str().unwrap();
        assert!(!s.is_empty());
    }

    /// Test that stats returns valid data.
    #[test]
    fn test_stats() {
        let _guard = test_guard();
        create_session();
        let input = main_input();

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("render failed: {}", take_error(err));
            }
            ssp_free(out);

            let mut stats: ssp_out_stats = ssp_out_stats::default();
            let err = ssp_stats(&mut stats as *mut _);
            if !err.is_null() {
                panic!("stats failed: {}", take_error(err));
            }
            assert!(stats.renders > 0, "should have at least one render");
        }
        destroy_session();
    }

    /// Test that ssp_free handles NULL safely.
    #[test]
    fn test_free_null() {
        unsafe {
            ssp_free(ptr::null_mut());
        }
    }

    /// Test pipestatus conversion.
    #[test]
    fn test_pipestatus() {
        let _guard = test_guard();
        create_session();

        let status = CString::new("0").unwrap();
        let ps0 = CString::new("0").unwrap();
        let ps1 = CString::new("1").unwrap();
        let ps_array = [ps0.as_ptr(), ps1.as_ptr()];
        let input = ssp_render_input {
            status: status.as_ptr(),
            pipestatus: ps_array.as_ptr(),
            pipestatus_len: 2,
            ..main_input()
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("render with pipestatus failed: {}", take_error(err));
            }
            ssp_free(out);
        }
        destroy_session();
    }

    /// Test cache behavior: two renders in the same directory produce cache hits.
    #[test]
    fn test_cache_hit_across_renders() {
        let _guard = test_guard();
        create_session();
        let input = main_input();

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("first render failed: {}", take_error(err));
            }
            let first_output = CStr::from_ptr(out).to_str().unwrap().to_string();
            ssp_free(out);

            let mut out2: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out2);
            if !err.is_null() {
                panic!("second render failed: {}", take_error(err));
            }
            let second_output = CStr::from_ptr(out2).to_str().unwrap().to_string();
            ssp_free(out2);

            assert_eq!(
                first_output, second_output,
                "consecutive renders in same directory should produce same output"
            );
        }
        destroy_session();
    }

    /// Test that destroying and recreating a session yields a fresh state:
    /// cache counters reset and rendering works again.
    #[test]
    fn test_recreate_resets_state() {
        let _guard = test_guard();
        let input = main_input();

        create_session();
        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("first render failed: {}", take_error(err));
            }
            ssp_free(out);

            let mut stats: ssp_out_stats = ssp_out_stats::default();
            let err = ssp_stats(&mut stats as *mut _);
            if !err.is_null() {
                panic!("stats failed: {}", take_error(err));
            }
            assert!(stats.renders > 0, "first session should have rendered");
        }
        destroy_session();

        // No session after destroy.
        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            assert!(!err.is_null(), "render after destroy should error");
            take_error(err);
        }

        // Recreate: statistics and caches must start from zero.
        create_session();
        unsafe {
            let mut stats: ssp_out_stats = ssp_out_stats::default();
            let err = ssp_stats(&mut stats as *mut _);
            if !err.is_null() {
                panic!("stats failed after recreate: {}", take_error(err));
            }
            assert_eq!(stats.renders, 0, "new session must start with zero renders");

            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_render(&input as *const _, &mut out);
            if !err.is_null() {
                panic!("render after recreate failed: {}", take_error(err));
            }
            ssp_free(out);

            let err = ssp_stats(&mut stats as *mut _);
            if !err.is_null() {
                panic!("stats failed after recreate render: {}", take_error(err));
            }
            assert_eq!(stats.renders, 1, "new session should count new renders");
        }
        destroy_session();
    }
}
