//! C FFI exports for Starship in-process prompt rendering.
//!
//! All public functions follow the `ssp_` prefix convention and use C ABI.
//! Heap strings returned to C are owned by Rust and must be freed with
//! `ssp_free`; `ssp_version` returns a static string instead.

use libc::c_char;
use starship::context::{Properties, Target};
use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;

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
pub struct ssp_stats {
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
// Session wrapper
// ---------------------------------------------------------------------------

/// Opaque session handle passed to C code.
pub struct SessionHandle {
    session: starship::session::Session,
    creator_pid: u32,
}

/// Check for fork: zsh forks for $(...), &, and pipelines, and the child
/// process inherits the tokio/rayon runtime in a corrupted state.
///
/// Returns an allocated error string from the enclosing `ffi_guard_error!`
/// closure when the current process is a forked child.
macro_rules! guard_fork {
    ($handle:expr) => {
        if unsafe { &*$handle }.creator_pid != std::process::id() {
            return error_string("refusing call in forked child process");
        }
    };
}

// ---------------------------------------------------------------------------
// Public C API
// ---------------------------------------------------------------------------

/// Create a new prompt rendering session.
///
/// Writes the new handle to `*out` on success and returns NULL. On failure
/// `*out` is set to NULL and an allocated error string is returned; free it
/// with `ssp_free()`.
///
/// The session persists across prompt renders within the same shell session.
///
/// # Safety
///
/// `out` must be NULL or point to writable `*mut SessionHandle` storage for
/// the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_session_create(out: *mut *mut SessionHandle) -> *mut c_char {
    ffi_guard_error!({
        if out.is_null() {
            return error_string("ssp_session_create: null out");
        }
        // SAFETY: `out` was checked non-null and is writable for this call.
        unsafe { *out = ptr::null_mut() };

        // Session creates its own scoped rayon pool (no global pool).
        // The pool is cleanly terminated when the session is destroyed.
        let handle = Box::new(SessionHandle {
            session: starship::session::Session::new(),
            creator_pid: std::process::id(),
        });
        // SAFETY: `out` is writable for this call.
        unsafe { *out = Box::into_raw(handle) };
        ptr::null_mut()
    })
}

/// Destroy a session previously created with `ssp_session_create`.
///
/// Returns NULL on success or an allocated error string (free with
/// `ssp_free()`). Passing NULL is a successful no-op.
///
/// # Safety
///
/// `handle` must be NULL or a live handle returned by `ssp_session_create`
/// that has not already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_session_destroy(handle: *mut SessionHandle) -> *mut c_char {
    ffi_guard_error!({
        if handle.is_null() {
            return ptr::null_mut();
        }
        guard_fork!(handle);
        // SAFETY: `handle` came from `ssp_session_create` and is destroyed once.
        unsafe {
            let _ = Box::from_raw(handle);
        }
        ptr::null_mut()
    })
}

/// Render a prompt for the given input parameters.
///
/// Returns NULL on success and writes a Rust-allocated prompt string to `*out`
/// (free it with `ssp_free()`). Returns an allocated error string on failure
/// and sets `*out` to NULL.
///
/// # Safety
///
/// `handle` must be a live session handle, `input` must point to a valid
/// `ssp_render_input`, and `out` must be NULL or point to writable
/// `*mut c_char` storage for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_session_render(
    handle: *mut SessionHandle,
    input: *const ssp_render_input,
    out: *mut *mut c_char,
) -> *mut c_char {
    ffi_guard_error!({
        if out.is_null() {
            return error_string("ssp_session_render: null argument");
        }

        // Always reset the caller's output slot before any other validation.
        // On failure the caller may otherwise keep a stale/dangling pointer
        // from a previous call.
        // SAFETY: `out` was checked non-null above and is writable for the
        // duration of this call.
        unsafe { *out = ptr::null_mut() };

        if handle.is_null() || input.is_null() {
            return error_string("ssp_session_render: null argument");
        }
        guard_fork!(handle);
        let handle = unsafe { &*handle };
        let input = unsafe { &*input };

        let properties = input.to_properties(input.target());
        let target = input.target();
        let output = handle.session.render(properties, target);

        unsafe {
            *out = string_into_c(output);
        }
        ptr::null_mut()
    })
}

/// Free a string previously returned by any fallible `ssp_*` function.
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

/// Retrieve cache performance statistics for a session.
///
/// Returns NULL on success and writes the snapshot to `*out`. Returns an
/// allocated error string on failure (free it with `ssp_free()`).
///
/// # Safety
///
/// `handle` must be a live session handle and `out` must be NULL or point to
/// writable `ssp_stats` storage for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ssp_session_stats(
    handle: *mut SessionHandle,
    out: *mut ssp_stats,
) -> *mut c_char {
    ffi_guard_error!({
        if handle.is_null() || out.is_null() {
            return error_string("ssp_session_stats: null argument");
        }
        guard_fork!(handle);
        let handle = unsafe { &*handle };
        let stats = handle.session.state().stats();
        let c_stats = ssp_stats {
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
        unsafe {
            *out = c_stats;
        }
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

    /// Create a session for tests, panicking if creation fails.
    fn new_session() -> *mut SessionHandle {
        let mut session: *mut SessionHandle = ptr::null_mut();
        let err = unsafe { ssp_session_create(&mut session) };
        if !err.is_null() {
            let msg = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { ssp_free(err) };
            panic!("session creation failed: {msg}");
        }
        assert!(!session.is_null(), "session creation returned NULL");
        session
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

    /// Test basic session creation and destruction.
    #[test]
    fn test_session_create_destroy() {
        let session = new_session();
        let err = unsafe { ssp_session_destroy(session) };
        assert!(err.is_null(), "destroy should succeed");
    }

    /// Test that destroying NULL is safe.
    #[test]
    fn test_session_destroy_null() {
        let err = unsafe { ssp_session_destroy(ptr::null_mut()) };
        assert!(err.is_null(), "destroying NULL is a successful no-op");
    }

    /// Test that a null out-slot on session creation returns an error.
    #[test]
    fn test_session_create_null_out() {
        let err = unsafe { ssp_session_create(ptr::null_mut()) };
        assert!(!err.is_null(), "null out should return an error");
        let msg = take_error(err);
        assert!(msg.contains("null"), "unexpected error: {msg}");
    }

    /// Test render with NULL arguments returns a call-local error string.
    #[test]
    fn test_render_null_args() {
        let session = new_session();

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, ptr::null(), &mut out);
            assert!(!err.is_null(), "null input should return an error");
            assert!(out.is_null(), "failed render should clear *out");
            let msg = take_error(err);
            assert!(msg.contains("null argument"), "unexpected error: {msg}");

            let err = ssp_session_render(ptr::null_mut(), ptr::null(), &mut out);
            assert!(!err.is_null(), "null handle should return an error");
            assert!(out.is_null(), "failed render should clear *out");
            take_error(err);

            assert!(ssp_session_destroy(session).is_null());
        }
    }

    /// Test rendering a main prompt.
    #[test]
    fn test_render_main_prompt() {
        let session = new_session();
        let status = CString::new("0").unwrap();
        let input = ssp_render_input {
            status: status.as_ptr(),
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
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out);
            if !err.is_null() {
                panic!("render failed: {}", take_error(err));
            }
            assert!(!out.is_null(), "output should be non-null");

            let output = CStr::from_ptr(out).to_str().unwrap();
            assert!(!output.is_empty(), "output should not be empty");

            ssp_free(out);
            assert!(ssp_session_destroy(session).is_null());
        }
    }

    /// Test rendering a right prompt.
    #[test]
    fn test_render_right_prompt() {
        let session = new_session();
        let input = ssp_render_input {
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
            target: 1, // Right
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out);
            if !err.is_null() {
                panic!("right prompt render failed: {}", take_error(err));
            }
            ssp_free(out);
            assert!(ssp_session_destroy(session).is_null());
        }
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
        let session = new_session();
        let input = main_input();

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out);
            if !err.is_null() {
                panic!("render failed: {}", take_error(err));
            }
            ssp_free(out);

            let mut stats: ssp_stats = ssp_stats::default();
            let err = ssp_session_stats(session, &mut stats as *mut _);
            if !err.is_null() {
                panic!("stats failed: {}", take_error(err));
            }
            assert!(stats.renders > 0, "should have at least one render");

            assert!(ssp_session_destroy(session).is_null());
        }
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
        let session = new_session();
        let status = CString::new("0").unwrap();
        let ps0 = CString::new("0").unwrap();
        let ps1 = CString::new("1").unwrap();
        let ps_array = [ps0.as_ptr(), ps1.as_ptr()];

        let input = ssp_render_input {
            status: status.as_ptr(),
            pipestatus: ps_array.as_ptr(),
            pipestatus_len: 2,
            terminal_width: 80,
            path: ptr::null(),
            logical_path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        };

        unsafe {
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out);
            if !err.is_null() {
                panic!("render with pipestatus failed: {}", take_error(err));
            }
            ssp_free(out);
            assert!(ssp_session_destroy(session).is_null());
        }
    }

    /// Test cache behavior: two renders in the same directory produce cache hits.
    #[test]
    fn test_cache_hit_across_renders() {
        let session = new_session();
        let input = main_input();

        unsafe {
            // First render.
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out);
            if !err.is_null() {
                panic!("first render failed: {}", take_error(err));
            }
            let first_output = CStr::from_ptr(out).to_str().unwrap().to_string();
            ssp_free(out);

            // Second render (within TTL, should use cached data).
            let mut out2: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(session, &input as *const _, &mut out2);
            if !err.is_null() {
                panic!("second render failed: {}", take_error(err));
            }
            let second_output = CStr::from_ptr(out2).to_str().unwrap().to_string();
            ssp_free(out2);

            assert_eq!(
                first_output, second_output,
                "consecutive renders in same directory should produce same output"
            );

            assert!(ssp_session_destroy(session).is_null());
        }
    }

    /// Test that every call returns its own error value: a failure on one
    /// session does not affect another session, and successful calls return
    /// NULL with no shared error slot to inspect.
    #[test]
    fn test_errors_are_call_local() {
        let first = new_session();
        let second = new_session();
        let input = main_input();

        unsafe {
            // Fail on the first session.
            let mut out: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(first, ptr::null(), &mut out);
            let msg = take_error(err);
            assert!(msg.contains("null argument"), "unexpected error: {msg}");
            assert!(out.is_null());

            // The second session can render successfully.
            let mut out2: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(second, &input as *const _, &mut out2);
            assert!(err.is_null(), "second session render should succeed");
            ssp_free(out2);

            // The first session can also render successfully afterwards.
            let mut out1: *mut c_char = ptr::null_mut();
            let err = ssp_session_render(first, &input as *const _, &mut out1);
            assert!(err.is_null(), "first session render should succeed");
            ssp_free(out1);

            assert!(ssp_session_destroy(first).is_null());
            assert!(ssp_session_destroy(second).is_null());
        }
    }
}
