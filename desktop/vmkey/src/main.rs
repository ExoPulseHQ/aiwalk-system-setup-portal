//! `aiwalk-sealed-vm [vault]`: sealed notes on a phone while Android's Linux Terminal is running. The Terminal's VM
//! takes every USB device plugged in while it runs, so the Android app no longer sees the key. This unlocks the vault
//! here instead (the key's PIN, then a touch) and answers Obsidian as the app does (phonehttp.rs), on port 47171,
//! which the Terminal forwards to the phone. `.sealed-token` gets "47171 <token>" so the plugin knows where to ask.
//! Unplugging the key or Ctrl+C locks: the group key lives only in this process.

#[cfg(target_os = "linux")]
#[path = "../../src/fidokey.rs"]
#[allow(dead_code)]
mod fidokey;
#[cfg(target_os = "linux")]
#[path = "../../src/agefido.rs"]
#[allow(dead_code)]
mod agefido;
#[cfg(target_os = "linux")]
#[path = "../../src/phonehttp.rs"]
mod phonehttp;

#[cfg(target_os = "linux")]
mod linux;

fn main() {
    #[cfg(target_os = "linux")]
    linux::main();
    #[cfg(not(target_os = "linux"))]
    eprintln!("aiwalk-sealed-vm runs in Android's Linux Terminal.");
}
