//! ggml's Metal code calls `@available`, which needs compiler-rt once a macOS deployment
//! target is set (Tauri sets one). rustc doesn't link it by itself, clang would.
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = Command::new("clang")
        .arg("--print-resource-dir")
        .output()
        .expect("cần clang (xcode-select --install)");
    let dir = String::from_utf8(out.stdout).expect("clang path is UTF-8");
    println!("cargo:rustc-link-search=native={}/lib/darwin", dir.trim());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
