#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// The app itself is app.rs; the Android build compiles the same file as a library (lib.rs).
include!("app.rs");
