//! Dynamic library loader and discovery for Google Pdfium.
//!
//! Enforces zero-trust secure library discovery using absolute system paths,
//! explicit environment variables, and `current_exe()`-relative locations only.
//! Insecure relative search paths (e.g. `./lib/` relative to CWD) are strictly
//! eliminated to prevent DLL preloading and untrusted workspace hijacking.

use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};

/// Attempts to discover and bind to dynamic `libpdfium.so`, `libpdfium.dylib`, or `pdfium.dll`.
pub fn discover_and_bind_pdfium() -> Option<Pdfium> {
    // 1. Explicit environment variable PDFIUM_LIB_PATH (points directly to library file)
    if let Ok(env_path_str) = std::env::var("PDFIUM_LIB_PATH") {
        let env_path = Path::new(&env_path_str);
        if env_path.is_file() {
            if let Ok(bindings) = Pdfium::bind_to_library(env_path) {
                log::info!(
                    "Bound to Pdfium via PDFIUM_LIB_PATH: {}",
                    env_path.display()
                );
                return Some(Pdfium::new(bindings));
            }
        }
    }

    // 2. Explicit environment variable PDFIUM_LIB_DIR (points to directory containing library)
    if let Ok(env_dir_str) = std::env::var("PDFIUM_LIB_DIR") {
        let env_dir = Path::new(&env_dir_str);
        if env_dir.is_dir() {
            for filename in &["libpdfium.so", "libpdfium.dylib", "pdfium.dll"] {
                let candidate = env_dir.join(filename);
                if candidate.is_file() {
                    if let Ok(bindings) = Pdfium::bind_to_library(&candidate) {
                        log::info!(
                            "Bound to Pdfium via PDFIUM_LIB_DIR: {}",
                            candidate.display()
                        );
                        return Some(Pdfium::new(bindings));
                    }
                }
            }
        }
    }

    // 3. Standard system dynamic linker search path
    if let Ok(bindings) = Pdfium::bind_to_system_library() {
        log::info!("Successfully initialized Pdfium from system dynamic library");
        return Some(Pdfium::new(bindings));
    }

    // 4. Secure executable-relative paths via std::env::current_exe()
    let mut candidate_paths: Vec<PathBuf> = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            for filename in &["libpdfium.so", "libpdfium.dylib", "pdfium.dll"] {
                candidate_paths.push(exe_dir.join(filename));
                candidate_paths.push(exe_dir.join("lib").join(filename));
            }
        }
    }

    // 5. User-level standard data directories (~/.local/share/kkpdf-zed/lib and ~/.local/lib)
    if let Ok(home_dir) = std::env::var("HOME") {
        if !home_dir.trim().is_empty() {
            let home = Path::new(&home_dir);
            for filename in &["libpdfium.so", "libpdfium.dylib"] {
                candidate_paths.push(home.join(".local/share/kkpdf-zed/lib").join(filename));
                candidate_paths.push(home.join(".local/lib").join(filename));
            }
        }
    }

    // 6. Well-known operating system shared library locations
    candidate_paths.extend([
        PathBuf::from("/usr/lib/libpdfium.so"),
        PathBuf::from("/usr/lib64/libpdfium.so"),
        PathBuf::from("/usr/local/lib/libpdfium.so"),
        PathBuf::from("/opt/homebrew/lib/libpdfium.dylib"),
        PathBuf::from("/usr/local/lib/libpdfium.dylib"),
        PathBuf::from("C:\\Windows\\System32\\pdfium.dll"),
    ]);

    // 7. Compile-time development workspace directory (cargo test / debug builds only)
    #[cfg(debug_assertions)]
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let manifest_path = Path::new(manifest_dir);
        for filename in &["libpdfium.so", "libpdfium.dylib", "pdfium.dll"] {
            candidate_paths.push(manifest_path.join("lib").join(filename));
        }
    }

    // Test discovered candidate paths
    for path in &candidate_paths {
        if path.is_file() {
            if let Ok(bindings) = Pdfium::bind_to_library(path) {
                log::info!("Successfully bound to Pdfium at {}", path.display());
                return Some(Pdfium::new(bindings));
            }
        }
    }

    log::warn!("Google Pdfium dynamic library not found in secure search paths");
    None
}
