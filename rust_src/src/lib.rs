//! C FFI bindings for Starship prompt rendering.
//!
//! This crate exposes a C-compatible API that allows shell modules (zsh, pwsh)
//! to render Starship prompts in-process without spawning a subprocess per render.
//!
//! # Safety
//!
//! All FFI functions use `catch_unwind` wrappers to prevent Rust panics from
//! unwinding across the FFI boundary. Errors are reported via return codes and
//! a thread-local error string accessible via `ssp_last_error()`.

pub mod ffi;
