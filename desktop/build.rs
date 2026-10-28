fn main() {
    // aiwalk-claude-vm, built for Android's Linux Terminal before the APK (release.yml), goes into the Android app
    // (linuxterm.rs puts it where the Terminal sees it). Without it, an empty file: setup.sh then downloads it.
    println!("cargo:rerun-if-env-changed=AIWALK_CLAUDE_VM");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("aiwalk-claude-vm");
    match std::env::var("AIWALK_CLAUDE_VM") {
        Ok(p) if !p.is_empty() => { println!("cargo:rerun-if-changed={p}"); std::fs::copy(&p, &out).expect("AIWALK_CLAUDE_VM names no file"); }
        _ => { let _ = std::fs::write(&out, b""); }
    }
    tauri_build::build()
}
