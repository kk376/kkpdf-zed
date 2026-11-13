# `kkpdf-zed`: Native PDF Viewer for Zed Editor

A native, high-performance PDF viewing engine and GPUI workspace item engineered for the [Zed Editor](https://zed.dev).

---

## 0. Architectural Identity & Delivery Model

> **Important**: This is a **native workspace crate**, not an installable WASM extension.  
> As of current Zed versions, Zed's WebAssembly extension API (`zed_extension_api`) contains no custom UI or file preview surfaces. `kkpdf-zed` is designed as a native crate to be registered within the Zed workspace (`crates/kkpdf_viewer` or integrated upstream into `zed-industries/zed`).

`extension.toml` in this repository is a **documentation stub** preserving metadata identity for a potential future extension surface or upstream contribution.

---

## 1. Core Architectural Pillars

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                             KKPDF-ZED CORE PIPELINE                              │
└──────────────────────────────────────────────────────────────────────────────────┘
                                        │
                      [PDF Document on Disk / Memory Buffer]
                                        │
                                        ▼
                  [Thread-Safe Document Handle (`document.rs`)]
                                        │
                                        ▼
                  [Serialized Native Pdfium Engine (`pdfium/`)]
                                        │
                                        ▼
                  [Memory-Budgeted LRU Cache (`cache.rs`)]
                                        │
                       (Visible Viewport + 1-Page Margin)
                                        │
                                        ▼
                [Luminosity Tone-Mapping Engine (`rasterizer.rs`)]
                                        │
                       (Inverts White/Black; Preserves Images)
                                        │
                                        ▼
                [Viewport State & Geometry Engine (`view.rs`)]
```

1. **Thread-Safe Serialization**: PDF parsing and rasterization are synchronized behind thread-safe handles, ready to be dispatched to background executor tasks in GPUI.
2. **Viewport Virtualization & LRU Caching**: Pages are rasterized on-demand with memory-budgeted caching (`PageLruCache`) to prevent unbounded RAM usage.
3. **Luminosity-Threshold Dark Mode**: Unlike simple blanket RGB inversion (which turns photos and charts into negative ghosts), `kkpdf-zed` implements selective tone mapping, remapping near-white canvas backgrounds to editor background tones and near-black text to theme light text, while preserving saturated color images.
4. **Scroll & Zoom Preserving Live Reload**: Watches the underlying PDF on disk (via file system notifications) and reloads seamlessly without resetting the user's scroll percentage or zoom level.

---

## 2. Scope Boundaries (v1)

- **Bitmap-Rendered Continuous Scroll**: Version 1 is engineered strictly as a high-fidelity, high-frame-rate read-only document viewer.
- **Excluded from v1**:
  - Text selection and copying.
  - In-document text search (`Ctrl+F`).
  - Interactive PDF form field editing.
- *Rationale*: Delivering an ultra-fast, stutter-free viewing experience with zero UI locks takes precedence. Text extraction and selection layer synchronization will be introduced in subsequent milestones.

---

## 3. Security Posture & Threat Model

- **In-Process Parsing**: PDF rasterization is performed via Google's native C++ [Pdfium](https://pdfium.googlesource.com/pdfium/) engine linked in-process with the editor.
- **Accepted Risk in v1**: Because Pdfium runs in-process without sandboxing (e.g., without separate process IPC or WebAssembly memory isolation), malformed or malicious PDFs could theoretically trigger memory vulnerabilities in the underlying C++ library.
- **Mitigation & Future Hardening**: Memory budgets are strictly capped, document handles are isolated behind thread boundaries, and a future sandboxed worker process model is planned for untrusted file browsing.

---

## 4. Repository Layout

```
kkpdf-zed/
├── Cargo.toml               # Native crate manifest (Rust Edition 2024, MSRV 1.88+)
├── rust-toolchain.toml      # Pinned stable toolchain with clippy & rustfmt
├── extension.toml           # Extension metadata stub
├── README.md                # Architecture, API & operational guide
├── SECURITY.md              # Zero-trust defensive threat model & security invariants
├── scripts/
│   └── download_pdfium.sh   # Automated dynamic library fetcher for native builds
├── examples/
│   └── viewer.rs            # Standalone interactive engine runner & benchmark harness
├── tests/
│   └── integration_tests.rs # Invariant test suite (tone mapping, header validation, LRU)
└── src/
    ├── lib.rs               # Crate entrypoint & re-exports
    ├── document.rs          # Thread-safe PdfDocument handle & header validation
    ├── rasterizer.rs        # Tone mapping & multi-scale background rasterization
    ├── cache.rs             # Memory-budgeted LRU cache with byte-level eviction
    ├── settings.rs          # Viewport policies, layout modes & zoom configuration
    ├── view.rs              # Viewport state machine, navigation & pan/zoom geometry
    ├── watcher.rs           # Live-reload debouncer & scroll/zoom state preservation
    ├── pdfium/              # Google Pdfium engine integration
    │   ├── mod.rs           # Engine abstraction, coordination & thread synchronization
    │   ├── loader.rs        # Zero-trust dynamic library discovery & mock fallback
    │   ├── render.rs        # Page & composite document rasterization routines
    │   ├── text.rs          # Text extraction & bounding box segmentation
    │   └── links.rs         # Hyperlink & URI annotation extraction
    └── ui/                  # GPUI user interface surfaces
        ├── mod.rs           # UI module exports
        └── toolbar.rs       # Zoom, fit-width/page & navigation controls
```

---

## 5. Local Development & Testing

**Toolchain Requirement**: Rust Edition 2024 (`rust-version = "1.88.0"` or newer).

Run unit and integration tests across the caching and rasterization engines:

```bash
# Run all unit and integration tests (mock fallback enabled when native library is absent)
cargo test

# Run tests against native Google Pdfium dynamic libraries:
bash scripts/download_pdfium.sh "${PWD}/lib"
PDFIUM_LIB_DIR="${PWD}/lib" cargo test

# Run standalone benchmark and interactive rendering runner:
cargo run --example viewer
```

---

## 6. Community Distributions & Downstream Builds

The wider Zed community has packaged and extended `kkpdf-zed` for end-user workflows:

- **[TendonT52/zed-with-pdf-viewer](https://github.com/TendonT52/zed-with-pdf-viewer)**: Provides prebuilt, ready-to-run macOS application bundles (`.dmg` for Apple Silicon `aarch64` and Intel `x86_64`) that track upstream Zed stable releases with native PDF rendering built in.
- **[HomoY/zed-with-pdf-viewer-synctex](https://github.com/HomoY/zed-with-pdf-viewer-synctex)**: Extends downstream Zed PDF integration with bidirectional **SyncTeX** support for LaTeX and Typst workflows (<kbd>Cmd</kbd>-click PDF to jump to TeX source; <kbd>Cmd</kbd>+<kbd>Option</kbd>+<kbd>J</kbd> to jump from editor to PDF viewport).

---

## 7. Prior Art & Acknowledgements

`kkpdf-zed` is a clean-room, native Rust implementation designed specifically for Zed's GPUI framework. We gratefully acknowledge the open source projects that inspired our workflow and interface concepts:

- **[Zed](https://zed.dev)** ([zed-industries/zed](https://github.com/zed-industries/zed)): The high-performance code editor and GPUI framework powering this experience (GPL-3.0 / Apache-2.0).
- **[pdfium-render](https://crates.io/crates/pdfium-render)**: Idiomatic Rust FFI bindings to Google Chrome's native Pdfium engine (Apache-2.0 / MIT).
- **[LaTeX-Workshop](https://github.com/James-Yu/LaTeX-Workshop)** by James Yu (MIT License): Inspired the live-reload debounce strategy and scroll-preserving document refresh workflows for TeX/Typst compilation.
- **[vscode-pdfviewer](https://github.com/tomoki1207/vscode-pdfviewer)** by tomoki1207 (MIT License): Inspired ergonomic zoom-level scaling increments and toolbar layout patterns.
- **[typst-preview](https://github.com/Enter-tainer/typst-preview)** by mgt (MIT License): Informed smooth previewer synchronization concepts.

*Note: No third-party source code was copied into this project; all GPUI painters, rasterizer pipelines, and caching layers are original native Rust implementations.*

---

## 8. License

This project is dual-licensed under either:

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))
- **GNU General Public License, Version 3.0 or later** ([LICENSE-GPL](LICENSE-GPL) or [https://www.gnu.org/licenses/gpl-3.0.html](https://www.gnu.org/licenses/gpl-3.0.html))

at your option.
