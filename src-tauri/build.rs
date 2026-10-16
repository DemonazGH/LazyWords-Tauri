use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=windows/autostart_cleanup.rs");
    println!("cargo:rerun-if-changed=windows/autostart-cleanup.wxs");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
        // WiX's SourceDir is the full main-EXE path, including its filename.
        let dll = out_dir
            .ancestors()
            .nth(3)
            .expect("Cargo profile directory")
            .join("lazywords.exe.autostart.dll");
        let status = Command::new(env::var_os("RUSTC").expect("RUSTC"))
            .args([
                "--edition=2021",
                "--crate-type=cdylib",
                "--crate-name=autostart_cleanup",
            ])
            .arg("windows/autostart_cleanup.rs")
            .arg("--target")
            .arg(env::var("TARGET").expect("TARGET"))
            .args(["-C", "opt-level=s", "-C", "target-feature=+crt-static"])
            .arg("-o")
            .arg(&dll)
            .status()
            .expect("compile MSI autostart cleanup DLL");
        assert!(
            status.success(),
            "MSI autostart cleanup DLL compilation failed"
        );
    }

    tauri_build::build()
}
