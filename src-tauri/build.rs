use std::path::Path;

fn main() {
    copy_native_dlls();
    tauri_build::build()
}

/// onnxruntime, DirectML and the Visual C++ runtime next to the app, the
/// examples and the test binaries (`deps`), so they run from `target`.
fn copy_native_dlls() {
    println!("cargo:rerun-if-changed=native/sherpa-onnx/VERSION.txt");
    let (Ok(out), Ok(profile)) = (std::env::var("OUT_DIR"), std::env::var("PROFILE")) else { return };
    let Some(profile_dir) = Path::new(&out).ancestors().find(|p| p.file_name().is_some_and(|n| n == profile.as_str())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir("native/sherpa-onnx") else { return };
    let dlls: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "dll")).collect();
    for dir in [profile_dir.to_path_buf(), profile_dir.join("examples"), profile_dir.join("deps")] {
        let _ = std::fs::create_dir_all(&dir);
        for dll in &dlls {
            let _ = std::fs::copy(dll, dir.join(dll.file_name().unwrap()));
        }
    }
}
