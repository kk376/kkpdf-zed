# Accessibility Statement

`kkpdf-zed` is committed to ensuring digital accessibility and ergonomic viewing comfort for all users, including individuals with low vision, photophobia, contrast sensitivity, motor impairments, and those navigating via keyboard or assistive technologies.

---

## Technical Standards & Accessibility Features

As a native PDF viewing engine and GPUI workspace item engineered for Zed Editor, `kkpdf-zed` incorporates the following accessibility-focused design principles:

### 1. Smart Luminosity-Threshold Dark Mode
Prolonged reading of stark white PDF canvases in low-light environments causes eye strain, astigmatic halation, and severe discomfort for users with photophobia or light sensitivity.

Unlike naive color inversion (`255 - C`), which turns photos, diagrams, and syntax-highlighted code blocks into unreadable negative ghosts, `kkpdf-zed` implements an **ITU-R BT.709 luminosity-preserving tone mapper** ([`src/rasterizer.rs`](src/rasterizer.rs)):
* **Paper & Ink Remapping**: Neutral canvas pixels ($S < 0.18$) are remapped using a linear lookup table: pure white paper (`#FFFFFF`) becomes dark editor surface (`#1E1E2E`), and black text (`#000000`) becomes crisp light text (`#CDD6F4`).
* **Image & Syntax Preservation**: Saturated color pixels ($S \ge 0.18$) like diagrams, photographs, and syntax highlights remain completely untouched, retaining their original colors and semantic meaning.

### 2. High-DPI Rendering & Scalable Typography
* **Retina / HiDPI Baseline**: Defaults to 144.0 DPI target rasterization scale ([`src/settings.rs`](src/settings.rs)), ensuring razor-sharp font glyphs and high edge contrast on high-density displays.
* **Granular Zoom Range**: Supports continuous magnification from 10% (`MIN_ZOOM = 0.1`) up to 2000% (`MAX_ZOOM = 20.0`) in ergonomic 1.15x steps (`ZOOM_STEP = 1.15`), alongside one-click automated fitting modes (`FitWidth`, `FitPage`).

### 3. Keyboard-First Navigation & Motor Accessibility
All primary viewport interactions operate fully through keyboard controls and GPUI action dispatchers without requiring mouse drags or pointer precision:
* **Page Navigation**: Next page, previous page, and direct numeric page jumps ([`src/view.rs`](src/view.rs)).
* **Viewport Zooming**: Incremental zoom in, zoom out, fit-to-width, and reset to actual size.
* **Layout Switching**: Continuous vertical scroll, single-page presentation, or two-page spread.

### 4. Text Extraction & Screen Reader Integration Roadmap
* **Underlying Text Engine**: The PDF engine extracts character bounding boxes, text streams, and hyperlink destinations directly from Google Pdfium ([`src/pdfium/text.rs`](src/pdfium/text.rs), [`src/pdfium/links.rs`](src/pdfium/links.rs)).
* **Assistive Tree Roadmap**: We are actively working toward exposing structured text spans directly into Zed's platform accessibility bridges (AT-SPI on Linux, NSAccessibility on macOS, and UI Automation on Windows).

---

## Known Limitations

* **Rasterized Viewport Surface (v1)**: In the current milestone, pages are rendered as GPU-composited bitmaps. Direct operating-system screen reader line-by-line text cursor tracking is under active development. Users requiring immediate text-to-speech synthesis should extract the text stream via auxiliary CLI tools while viewing.

---

## Reporting Accessibility Barriers

If you encounter accessibility barriers, visual strain issues, or compatibility challenges with assistive tools in `kkpdf-zed`, please contact us:

* **GitHub Issues**: [Open an issue on GitHub](https://github.com/kk376/kkpdf-zed/issues/new) using the `accessibility` label.
* **Direct Contact**: Email Kushagra Kumar at [kkushagra86@gmail.com](mailto:kkushagra86@gmail.com).

Please include:
1. Your operating system, architecture, and display environment (e.g. Wayland, macOS Metal, Windows).
2. The version of Zed and `kkpdf-zed`.
3. Any assistive technologies or contrast profiles in use.
4. A description of the expected versus observed reading experience.
