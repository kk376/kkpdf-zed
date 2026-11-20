# Contributing to `kkpdf-zed`

Thank you for contributing to `kkpdf-zed`. This guide covers local environment setup, architecture, testing protocols, and core engineering invariants.

---

## 1. Project Identity & Architecture

`kkpdf-zed` is a high-performance native PDF viewing engine and GPUI workspace item designed for the [Zed Editor](https://zed.dev).

Key architectural layers:
* **FFI Engine Layer (`src/pdfium/`)**: Wraps Google's native C++ Pdfium library via `pdfium-render`. All engine interactions are strictly serialized behind `SerializedPdfium` under `parking_lot::Mutex`.
* **LRU Cache Layer (`src/cache.rs`)**: Manages rasterized page bitmaps with an $O(1)$ memory-budgeted cache (`PageLruCache`) capped at `DEFAULT_MEMORY_BUDGET_BYTES` (256 MB default).
* **Rasterizer & Tone Mapping (`src/rasterizer.rs`)**: Implements ITU-R BT.709 luminosity-preserving dark mode remapping and mock rasterization for headless environments.
* **View & Navigation State (`src/view.rs`, `src/settings.rs`)**: Encapsulates zoom calculation, scroll virtualization, page layouts, and container geometry.
* **Hot-Reload Watcher (`src/watcher.rs`)**: Debounces file change events for smooth compilation workflows (LaTeX, Typst) while preserving exact scroll and zoom state.

---

## 2. Prerequisites & Toolchain

* **Rust Toolchain**: Rust Edition 2024 with MSRV **1.88.0** or later (configured in `rust-toolchain.toml`).
* **Pdfium Dynamic Library** (optional for test suite; required for live rendering):
  * Linux: `libpdfium.so`
  * macOS: `pdfium.dylib`
  * Windows: `pdfium.dll`
* **Automated Library Fetcher**:
  ```bash
  # Downloads prebuilt verified Google Pdfium binaries to ./lib
  bash scripts/download_pdfium.sh "${PWD}/lib"
  export PDFIUM_LIB_DIR="${PWD}/lib"
  ```
* **Headless / CI Compatibility**: Standard `cargo test` builds and executes cleanly **without** the native Pdfium library installed, utilizing synthetic headless rasterization fallback.

---

## 3. Development Workflow

### Build from Source

```bash
git clone https://github.com/kk376/kkpdf-zed.git
cd kkpdf-zed
cargo build
```

### Running Tests

The test suite validates LRU stress eviction, tone mapper luminosity invariants, PDF magic-byte validation, and debounce state preservation:

```bash
# Run unit and integration tests (mock fallback enabled)
cargo test --all-targets

# Run tests against native Google Pdfium binaries
PDFIUM_LIB_DIR="${PWD}/lib" cargo test --all-targets
```

### Static Analysis & Formatting

All pull requests must pass formatting and clippy linter checks without warnings:

```bash
# Check code formatting (Edition 2024 style)
cargo fmt --check

# Check clippy linter with strict warning denial
cargo clippy --all-targets -- -D warnings
```

---

## 4. Secure Coding & Architectural Invariants

Contributors must uphold the following invariants:

1. **Undocumented Unsafe Prohibition**:
   - Enforced via `#![deny(clippy::undocumented_unsafe_blocks)]`.
   - Every `unsafe` block must include an explicit `// SAFETY:` rationale documenting how pointer validity, alignment, and thread safety are guaranteed.
2. **Zero Debug or Incomplete Code**:
   - Enforced via `#![deny(clippy::dbg_macro)]` and `#![deny(clippy::todo)]`.
   - No placeholder functions or fake stubs in production paths.
3. **Thread-Safe Serialization**:
   - Direct FFI access to Pdfium is non-thread-safe. Never bypass `SerializedPdfium` or acquire raw pointers across threads without serialization.
4. **Memory Budgeting**:
   - All bitmap allocations must go through `PageLruCache`. Never bypass memory caps to prevent Out-Of-Memory (OOM) conditions during large document rendering.
5. **Magic Byte Validation**:
   - Any document intake path must validate the `%PDF-` header via `validate_pdf_magic_bytes` before dispatching to the C++ parser.
6. **Luminosity Preservation**:
   - Changes to `src/rasterizer.rs` must preserve saturated color channels ($S \ge \text{threshold}$) to prevent inverted photo-negatives on figures and syntax-highlighted diagrams.

---

## 5. Submitting Pull Requests

1. **Branch Naming**: Use descriptive prefixes: `feat/*`, `fix/*`, `perf/*`, `docs/*`, `ci/*`.
2. **Commit Messages**: Follow Conventional Commits format (e.g. `feat(rasterizer): ...`, `fix(cache): ...`).
3. **Minimal Blast Radius**: Keep diffs surgical and focused strictly on the intended change.
4. **Test Coverage**: Add corresponding unit tests in the module or regression tests in `tests/integration_tests.rs`.
5. **PR Checklist**: Fill in [.github/pull_request_template.md](.github/pull_request_template.md) completely when submitting.
