//! Link against the system GSM/AMR codec libraries.
//!
//! Linux (Debian) ships these as runtime-only packages: only versioned shared
//! objects (`libgsm.so.1`, `libopencore-amrnb.so.0`), no `-dev` headers and no
//! unversioned `.so` a normal `-lgsm` would resolve — so we symlink a linkable
//! name into OUT_DIR. macOS (Homebrew) provides correctly-named `.dylib`s, so we
//! just point the linker at the brew lib dir. We never need headers (the FFI
//! decls are hand-written in `src/ffi.rs`).
//!
//! Install:
//!   Debian: `apt install libgsm1 libopencore-amrnb0`
//!   macOS:  `brew install libgsm opencore-amr`

use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let is_macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");

    let search_dirs: &[&str] = if is_macos {
        &[
            "/opt/homebrew/lib",            // Apple Silicon Homebrew
            "/usr/local/lib",               // Intel Homebrew
            "/opt/homebrew/opt/libgsm/lib",
            "/opt/homebrew/opt/opencore-amr/lib",
            "/usr/local/opt/libgsm/lib",
            "/usr/local/opt/opencore-amr/lib",
        ]
    } else {
        &[
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib",
            "/lib/x86_64-linux-gnu",
            "/usr/local/lib",
        ]
    };

    // (link name, linux sonames, macos sonames)
    let libs: &[(&str, &[&str], &[&str])] = &[
        ("gsm", &["libgsm.so.1", "libgsm.so"], &["libgsm.dylib"]),
        (
            "opencore-amrnb",
            &["libopencore-amrnb.so.0", "libopencore-amrnb.so"],
            &["libopencore-amrnb.dylib"],
        ),
    ];

    for (link_name, linux_names, macos_names) in libs {
        let sonames: &[&str] = if is_macos { macos_names } else { linux_names };
        let full = sonames
            .iter()
            .flat_map(|so| search_dirs.iter().map(move |d| PathBuf::from(d).join(so)))
            .find(|p| p.exists())
            .unwrap_or_else(|| {
                panic!(
                    "dov-codec: could not find `{link_name}` ({sonames:?}) in {search_dirs:?}.\n\
                     Install it — Debian: `apt install libgsm1 libopencore-amrnb0`; \
                     macOS: `brew install libgsm opencore-amr`."
                )
            });

        if is_macos {
            // Homebrew's `.dylib` is already a linkable name.
            println!("cargo:rustc-link-search=native={}", full.parent().unwrap().display());
        } else {
            // Symlink a `-l`-resolvable name next to nothing else in OUT_DIR.
            let link_path = out_dir.join(format!("lib{link_name}.so"));
            let _ = std::fs::remove_file(&link_path);
            std::os::unix::fs::symlink(&full, &link_path).unwrap_or_else(|e| {
                panic!("dov-codec: failed to symlink {link_path:?} -> {full:?}: {e}")
            });
            println!("cargo:rustc-link-search=native={}", out_dir.display());
        }
        println!("cargo:rustc-link-lib=dylib={link_name}");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
