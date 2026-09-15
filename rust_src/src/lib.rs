//! C FFI bindings for Starship prompt rendering.
//!
//! This crate exposes a C-compatible API that allows shell modules (zsh, pwsh)
//! to render Starship prompts in-process without spawning a subprocess per render.
//!
//! # Safety
//!
//! All FFI functions use `catch_unwind` wrappers to prevent Rust panics from
//! unwinding across the FFI boundary.
//!
//! Fallible exports return `char *`: NULL means success, and a non-NULL value
//! is an allocated error string the caller must release with `ssp_free()`.
//! There are no global or per-session error slots to read afterwards.

pub mod ffi;
