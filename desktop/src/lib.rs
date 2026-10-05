//! The Android app: Tauri loads this library on a phone (`run`, in app.rs). On a computer the crate is empty and
//! main.rs is the program; both compile the same app.rs.
#![cfg(target_os = "android")]
// the desktop-only commands and command-line tools stay in app.rs, unused here
#![allow(dead_code, unused_imports)]

include!("app.rs");
