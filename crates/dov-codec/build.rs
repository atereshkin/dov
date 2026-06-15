//! Link against the system GSM/AMR codec libraries.
//!
//! Debian ships these as runtime-only packages: only the versioned shared
//! objects (`libgsm.so.1`, `libopencore-amrnb.so.0`) are present, with no
//! `-dev` headers and no unversioned `.so` symlink that a normal `-lgsm`
//! would resolve. We don't need headers (the FFI decls are hand-written in
//! `src/ffi.rs`), but we do need a linkable name. So we create unversioned
//! symlinks in OUT_DIR pointing at the versioned `.so` and link against those.

use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    let search_dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
        "/usr/local/lib",
    ];

    // (link name passed to `-l`, candidate sonames to resolve against)
    let libs: &[(&str, &[&str])] = &[
        ("gsm", &["libgsm.so.1", "libgsm.so"]),
        (
            "opencore-amrnb",
            &["libopencore-amrnb.so.0", "libopencore-amrnb.so"],
        ),
    ];

    for (link_name, sonames) in libs {
        let full = sonames
            .iter()
            .flat_map(|so| search_dirs.iter().map(move |d| PathBuf::from(d).join(so)))
            .find(|p| p.exists())
            .unwrap_or_else(|| {
                panic!("dov-codec: could not locate a shared object for `{link_name}` (looked for {sonames:?} in {search_dirs:?}). Install the corresponding runtime package.")
            });

        let link_path = out_dir.join(format!("lib{link_name}.so"));
        let _ = std::fs::remove_file(&link_path);
        std::os::unix::fs::symlink(&full, &link_path).unwrap_or_else(|e| {
            panic!("dov-codec: failed to symlink {link_path:?} -> {full:?}: {e}")
        });

        println!("cargo:rustc-link-lib=dylib={link_name}");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rerun-if-changed=build.rs");
}
