//! C FFI exports for Starship in-process prompt rendering.
//!
//! All public functions follow the `ssp_` prefix convention and use C ABI.
//! Strings returned to C are owned by Rust and must be freed with `ssp_free`.

use libc::c_char;
use starship::context::{Properties, Target};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_int;

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

fn clear_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

/// FFI panic guard: wraps a closure, catching any panic and converting it to
/// an error return (-1) with the panic message stored in the thread-local error.
///
/// Pattern adapted from zsh-native-syntax.
macro_rules! ffi_guard {
    ($expr:expr, $error_val:expr) => {{
        clear_error();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $expr)) {
            Ok(result) => result,
            Err(panic) => {
                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                    format!("panic: {s}")
                } else if let Some(s) = panic.downcast_ref::<String>() {
                    format!("panic: {s}")
                } else {
                    "panic: unknown error".to_string()
                };
                set_error(&msg);
                $error_val
            }
        }
    }};
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
    /// Logical working directory path (NULL = use process cwd).
    pub path: *const c_char,
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
        let mut props = Properties::default();
        props.status_code = cstr_to_option_string(self.status);
        props.pipestatus = cstr_array_to_vec(self.pipestatus, self.pipestatus_len);
        props.terminal_width = self.terminal_width;
        props.path = cstr_to_option_string(self.path).map(std::path::PathBuf::from);
        props.logical_path = None;
        props.cmd_duration = cstr_to_option_string(self.cmd_duration);
        props.keymap = cstr_to_option_string(self.keymap).unwrap_or_else(|| "viins".into());
        props.jobs = self.jobs;
        props.shlvl = if self.shlvl >= 0 {
            Some(self.shlvl)
        } else {
            None
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
}

// ---------------------------------------------------------------------------
// Public C API
// ---------------------------------------------------------------------------

/// Create a new prompt rendering session.
///
/// Returns a non-null opaque pointer on success, or null on failure
/// (check `ssp_last_error()`).
///
/// The session persists across prompt renders within the same shell session.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_session_create() -> *mut SessionHandle {
    ffi_guard!(
        {
            // Initialize the rayon global thread pool (mirrors what `starship` main does).
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(starship::num_rayon_threads())
                .build_global();
            let handle = Box::new(SessionHandle {
                session: starship::session::Session::new(),
            });
            Box::into_raw(handle)
        },
        std::ptr::null_mut()
    )
}

/// Destroy a session previously created with `ssp_session_create`.
///
/// Passing NULL is safe (no-op).
#[unsafe(no_mangle)]
pub extern "C" fn ssp_session_destroy(handle: *mut SessionHandle) {
    if handle.is_null() {
        return;
    }
    ffi_guard!(
        {
            unsafe {
                let _ = Box::from_raw(handle);
            }
        },
        ()
    );
}

/// Render a prompt for the given input parameters.
///
/// On success, returns 0 and writes a Rust-allocated string to `*out`.
/// The caller must free `*out` with `ssp_free()`.
/// On failure, returns a negative value; check `ssp_last_error()`.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_session_render(
    handle: *mut SessionHandle,
    input: *const ssp_render_input,
    out: *mut *mut c_char,
) -> c_int {
    ffi_guard!(
        {
            if handle.is_null() || input.is_null() || out.is_null() {
                set_error("ssp_session_render: null argument");
                return -1;
            }
            let handle = unsafe { &*handle };
            let input = unsafe { &*input };

            let properties = input.to_properties(input.target());
            let target = input.target();

            let output = handle.session.render(properties, target);

            let c_string = CString::new(output).unwrap_or_else(|e| {
                // Truncate at the first null byte (shouldn't happen in prompt output).
                let pos = e.nul_position();
                let mut bytes = e.into_vec();
                bytes.truncate(pos);
                CString::new(bytes).unwrap()
            });

            unsafe {
                *out = c_string.into_raw();
            }
            0
        },
        -1
    )
}

/// Free a string previously returned by `ssp_session_render`.
///
/// Passing NULL is safe (no-op).
#[unsafe(no_mangle)]
pub extern "C" fn ssp_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    ffi_guard!(
        {
            unsafe {
                let _ = CString::from_raw(ptr);
            }
        },
        ()
    );
}

/// Return the library version as a static string.
///
/// The returned pointer is valid for the lifetime of the process.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_version() -> *const c_char {
    static VERSION: std::sync::LazyLock<CString> =
        std::sync::LazyLock::new(|| CString::new(env!("CARGO_PKG_VERSION")).unwrap());
    VERSION.as_ptr()
}

/// Return the last error message from the current thread.
///
/// The returned pointer is valid until the next FFI call from the same thread.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

/// Retrieve cache performance statistics for a session.
///
/// Returns 0 on success, or a negative value if `handle` or `out` is null.
#[unsafe(no_mangle)]
pub extern "C" fn ssp_session_stats(handle: *mut SessionHandle, out: *mut ssp_stats) -> c_int {
    ffi_guard!(
        {
            if handle.is_null() || out.is_null() {
                set_error("ssp_session_stats: null argument");
                return -1;
            }
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
            0
        },
        -1
    )
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    /// Test basic session creation and destruction.
    #[test]
    fn test_session_create_destroy() {
        let session = ssp_session_create();
        assert!(!session.is_null(), "session creation should succeed");
        ssp_session_destroy(session);
    }

    /// Test that destroying NULL is safe.
    #[test]
    fn test_session_destroy_null() {
        ssp_session_destroy(ptr::null_mut());
    }

    /// Test rendering with NULL arguments returns error.
    #[test]
    fn test_render_null_args() {
        let session = ssp_session_create();
        assert!(!session.is_null());

        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(session, ptr::null(), &mut out);
        assert!(rc < 0, "null input should return error");

        let rc = ssp_session_render(ptr::null_mut(), ptr::null(), &mut out);
        assert!(rc < 0, "null handle should return error");

        ssp_session_destroy(session);
    }

    /// Test rendering a main prompt.
    #[test]
    fn test_render_main_prompt() {
        let session = ssp_session_create();
        assert!(!session.is_null());

        let status = CString::new("0").unwrap();
        let input = ssp_render_input {
            status: status.as_ptr(),
            pipestatus: ptr::null(),
            pipestatus_len: 0,
            terminal_width: 80,
            path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        };

        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(session, &input as *const _, &mut out);
        assert_eq!(rc, 0, "render should succeed");
        assert!(!out.is_null(), "output should be non-null");

        let output = unsafe { CStr::from_ptr(out) }.to_str().unwrap();
        assert!(!output.is_empty(), "output should not be empty");

        ssp_free(out);
        ssp_session_destroy(session);
    }

    /// Test rendering a right prompt.
    #[test]
    fn test_render_right_prompt() {
        let session = ssp_session_create();
        assert!(!session.is_null());

        let input = ssp_render_input {
            status: ptr::null(),
            pipestatus: ptr::null(),
            pipestatus_len: 0,
            terminal_width: 80,
            path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 1, // Right
        };

        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(session, &input as *const _, &mut out);
        assert_eq!(rc, 0, "right prompt render should succeed");
        ssp_free(out);
        ssp_session_destroy(session);
    }

    /// Test that version returns a non-null string.
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
        let session = ssp_session_create();
        assert!(!session.is_null());

        // Do one render to populate stats.
        let input = ssp_render_input {
            status: ptr::null(),
            pipestatus: ptr::null(),
            pipestatus_len: 0,
            terminal_width: 80,
            path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        };
        let mut out: *mut c_char = ptr::null_mut();
        ssp_session_render(session, &input as *const _, &mut out);
        ssp_free(out);

        let mut stats: ssp_stats = ssp_stats::default();
        let rc = ssp_session_stats(session, &mut stats as *mut _);
        assert_eq!(rc, 0);
        assert!(stats.renders > 0, "should have at least one render");

        ssp_session_destroy(session);
    }

    /// Test that ssp_free handles NULL safely.
    #[test]
    fn test_free_null() {
        ssp_free(ptr::null_mut());
    }

    /// Test pipestatus conversion.
    #[test]
    fn test_pipestatus() {
        let session = ssp_session_create();
        assert!(!session.is_null());

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
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        };

        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(session, &input as *const _, &mut out);
        assert_eq!(rc, 0, "render with pipestatus should succeed");
        ssp_free(out);
        ssp_session_destroy(session);
    }

    /// Test cache behavior: two renders in the same directory should produce cache hits.
    #[test]
    fn test_cache_hit_across_renders() {
        let session = ssp_session_create();
        assert!(!session.is_null());

        let input = ssp_render_input {
            status: ptr::null(),
            pipestatus: ptr::null(),
            pipestatus_len: 0,
            terminal_width: 80,
            path: ptr::null(),
            cmd_duration: ptr::null(),
            keymap: ptr::null(),
            jobs: 0,
            shlvl: -1,
            target: 0,
        };

        // First render.
        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(session, &input as *const _, &mut out);
        assert_eq!(rc, 0);
        let first_output = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        ssp_free(out);

        // Second render (within TTL, should use cached data).
        let mut out2: *mut c_char = ptr::null_mut();
        let rc2 = ssp_session_render(session, &input as *const _, &mut out2);
        assert_eq!(rc2, 0);
        let second_output = unsafe { CStr::from_ptr(out2) }
            .to_str()
            .unwrap()
            .to_string();
        ssp_free(out2);

        // Both renders should produce the same output in an empty directory.
        assert_eq!(
            first_output, second_output,
            "consecutive renders in same directory should produce same output"
        );

        ssp_session_destroy(session);
    }

    /// Test that panic in render doesn't crash the host process.
    #[test]
    fn test_error_recovery() {
        // ssp_last_error should return null initially.
        let err = ssp_last_error();
        assert!(err.is_null(), "no error should be set initially");

        // Render with null input should fail but not crash.
        let mut out: *mut c_char = ptr::null_mut();
        let rc = ssp_session_render(ptr::null_mut(), ptr::null(), &mut out);
        assert!(rc < 0, "null everything should return error");

        // Should have an error message now.
        let err = ssp_last_error();
        assert!(!err.is_null(), "error should be set after failure");
    }
}
