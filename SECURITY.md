# Security Policy & Defensive Threat Model

`kkpdf-zed` adheres to the **17-Category Vibe-Check Zero-Trust Defensive Engineering Standard**.

---

## 1. Threat Model & Security Posture

### A. In-Process C++ Pdfium Execution
- **Architecture**: `kkpdf-zed` dynamically binds to Google's C++ Pdfium rendering engine.
- **Risk**: Malformed or weaponized PDF exploit payloads targeting memory vulnerabilities (heap buffer overflows, use-after-free) in PDF parsing libraries.
- **Mitigation & Invariants**:
  - All FFI calls to Pdfium are wrapped in `SerializedPdfium` under a `parking_lot::Mutex`, guaranteeing strict serial access across threads.
  - Pre-flight magic-byte validation (`%PDF-` header check) rejects non-PDF payloads prior to invoking parser state machines.
  - In a full Zed integration, Pdfium rendering runs on background executor tasks, preventing UI thread blockage and isolating crash surfaces.

### B. Memory Budgeting & Denial-of-Service (DoS)
- **Risk**: Decompression bombs or massive 10,000-page PDF documents consuming gigabytes of RAM leading to Out-Of-Memory (OOM) editor crashes.
- **Mitigation & Invariants**:
  - Viewport-bounded rendering: Only pages visible on screen or in prefetch radius are rasterized.
  - Memory-budgeted LRU cache (`PageLruCache`): Bounded by `DEFAULT_MEMORY_BUDGET_BYTES` (256 MB default). Automatic byte-size eviction purges oldest bitmaps when total allocation exceeds the budget.
  - Bitmaps larger than the total cache budget are bypassed rather than overflowing system memory.

### C. File System & Hot-Reload Boundaries
- **Risk**: Malicious symlink loops or continuous file rewrite flood attacks exhausting CPU cycles.
- **Mitigation & Invariants**:
  - Debounced file watching (`PdfReloadDebouncer`): Coalesces rapid filesystem events (e.g. from `latexmk` or `typst watch`) with configurable debounce windows (200ms default).
  - Atomic reload lock: State snapshots are captured before re-reading bytes, ensuring corrupted partial writes fail closed without corrupting the active viewer state.

---

## 2. Secure Coding Invariants

1. **Undocumented Unsafe Prohibition**: `#![deny(clippy::undocumented_unsafe_blocks)]` is strictly enforced. Every `unsafe` block must include explicit `// SAFETY:` rationale documenting invariant safety proofs.
2. **Zero Debug / Todo Leftovers**: `#![deny(clippy::dbg_macro)]` and `#![deny(clippy::todo)]` are enforced across all build targets.
3. **No Unchecked Unwraps**: Production rendering routines use `anyhow::Result` context chaining and fallback gracefully if library bindings are unavailable.

---

## 3. Reporting a Vulnerability

If you discover a potential security vulnerability in `kkpdf-zed`:

- **Email**: Kushagra Kumar ([kkushagra86@gmail.com](mailto:kkushagra86@gmail.com))
- **Response SLA**: Initial triage within 24 hours.
- Please do not open public issues detailing reproducible exploit payloads until a patch is released.
