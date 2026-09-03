//! High-level Pdfium engine abstraction, lifecycle manager, and submodules.
//!
//! Provides thread-safe document rendering via Google Pdfium C++ engine,
//! with strict error reporting when native dynamic libraries are missing,
//! zero-trust library discovery, and explicit opt-in mock fallbacks for testing.

pub mod links;
pub mod loader;
pub mod render;
pub mod text;

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::cache::RenderedPage;
use crate::document::{PageDimensions, PdfDocument};
use crate::rasterizer::RasterizerOptions;

pub use links::PdfLinkAnnotation;
pub use render::{
    PdfDocumentRenderResult, PdfPageRenderResult, MAX_COMPOSITE_BUFFER_BYTES, MAX_COMPOSITE_HEIGHT,
    MAX_COMPOSITE_PAGES, MAX_PAGE_DIMENSION,
};
pub use text::PdfTextSegment;

/// Internal wrapper granting thread-safe synchronization to Pdfium handle.
pub(crate) struct SerializedPdfium(pub(crate) Option<Pdfium>);

// SAFETY: All calls to the internal Pdfium C++ FFI handle are strictly guarded
// by a Mutual Exclusion (Mutex) lock, guaranteeing serialized single-thread
// execution across asynchronous worker threads without race conditions.
unsafe impl Send for SerializedPdfium {}

// SAFETY: All calls to the internal Pdfium C++ FFI handle are strictly guarded
// by a Mutual Exclusion (Mutex) lock, guaranteeing synchronized multi-thread access.
unsafe impl Sync for SerializedPdfium {}

static GLOBAL_PDFIUM: OnceLock<Arc<Mutex<SerializedPdfium>>> = OnceLock::new();

/// Validates that the input byte slice starts with the standard PDF magic header (`%PDF-`).
/// Protects against sending arbitrary or malformed data into native C++ parser entry points.
#[inline]
pub fn validate_pdf_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 5 || &bytes[0..5] != b"%PDF-" {
        anyhow::bail!("Invalid PDF header magic bytes: expected %PDF-");
    }
    Ok(())
}

/// Detailed page metadata, text content, hyperlinks, and text segment bounding boxes.
#[derive(Debug, Clone, Default)]
pub struct PdfPageDetails {
    pub page_index: usize,
    pub width_pt: f32,
    pub height_pt: f32,
    pub text: String,
    pub links: Vec<PdfLinkAnnotation>,
    pub text_segments: Vec<PdfTextSegment>,
}

/// Detailed document metadata including page-by-page text, segments, and hyperlink annotations.
#[derive(Debug, Clone, Default)]
pub struct PdfDocumentDetails {
    pub total_pages: usize,
    pub full_text: String,
    pub pages: Vec<PdfPageDetails>,
}

/// Thread-safe wrapper around a Pdfium instance.
#[derive(Clone)]
pub struct PdfiumEngine {
    pub(crate) inner: Arc<Mutex<SerializedPdfium>>,
    pub(crate) allow_mock_fallback: bool,
}

impl Default for PdfiumEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfiumEngine {
    /// Creates a new engine handle, sharing the process-level singleton instance.
    ///
    /// By default, missing native libraries return an explicit `Err` rather than
    /// silently producing synthetic mock drawings. Mock fallback can be enabled
    /// by setting the `PDFIUM_MOCK_FALLBACK=1` environment variable.
    pub fn new() -> Self {
        let allow_mock_fallback = std::env::var("PDFIUM_MOCK_FALLBACK")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let inner = GLOBAL_PDFIUM
            .get_or_init(|| {
                Arc::new(Mutex::new(SerializedPdfium(
                    loader::discover_and_bind_pdfium(),
                )))
            })
            .clone();

        Self {
            inner,
            allow_mock_fallback,
        }
    }

    /// Creates an engine handle with synthetic mock fallback explicitly permitted (for test harnesses).
    pub fn with_mock_fallback() -> Self {
        let mut engine = Self::new();
        engine.allow_mock_fallback = true;
        engine
    }

    /// Creates an engine handle with no native library loaded and mock fallback enabled (for unit testing mock pipelines).
    pub fn mock() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SerializedPdfium(None))),
            allow_mock_fallback: true,
        }
    }

    /// Explicitly enables or disables mock fallback for this engine handle.
    pub fn set_allow_mock_fallback(&mut self, allow: bool) {
        self.allow_mock_fallback = allow;
    }

    /// True if native Pdfium C++ bindings are actively loaded.
    pub fn is_native_available(&self) -> bool {
        self.inner.lock().0.is_some()
    }

    /// Loads a PDF from disk path and returns document metadata.
    pub fn load_document_from_path(&self, path: &Path) -> Result<PdfDocument> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("Failed to read PDF file at {:?}", path))?;
        self.load_document_from_bytes(&bytes, Some(path.to_path_buf()))
    }

    /// Loads a PDF from an in-memory byte slice.
    pub fn load_document_from_bytes(
        &self,
        bytes: &[u8],
        path: Option<PathBuf>,
    ) -> Result<PdfDocument> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            let doc = pdfium
                .load_pdf_from_byte_slice(bytes, None)
                .context("Pdfium failed to parse PDF document byte slice")?;

            let mut pages = Vec::new();
            for page in doc.pages().iter() {
                let width = page.width().value;
                let height = page.height().value;
                pages.push(PageDimensions::new(width, height));
            }

            let title = doc
                .metadata()
                .get(PdfDocumentMetadataTagType::Title)
                .map(|s| s.value().to_string());

            Ok(PdfDocument::new(path, pages, title))
        } else if self.allow_mock_fallback {
            PdfDocument::from_bytes(bytes, path)
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }

    /// Rasterizes a specific page from file bytes into an RGBA frame buffer.
    pub fn render_page_from_bytes(
        &self,
        bytes: &[u8],
        page_index: usize,
        options: RasterizerOptions,
    ) -> Result<RenderedPage> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            render::render_single_page(pdfium, bytes, page_index, options)
        } else if self.allow_mock_fallback {
            render::render_mock_single_page(page_index, options)
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }

    /// Rasterizes a bounded viewport range of pages from file bytes into a continuous vertical layout.
    pub fn render_document_range_from_bytes(
        &self,
        bytes: &[u8],
        start_page: usize,
        page_budget: usize,
        options: RasterizerOptions,
        page_gap: u32,
    ) -> Result<RenderedPage> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            render::render_document_range(pdfium, bytes, start_page, page_budget, options, page_gap)
        } else if self.allow_mock_fallback {
            render::render_mock_document_range(bytes, start_page, page_budget, options, page_gap)
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }

    /// Rasterizes pages from file bytes into a continuous vertical layout,
    /// bounded by `MAX_COMPOSITE_PAGES` to prevent multi-GB memory exhaustion.
    pub fn render_document_from_bytes(
        &self,
        bytes: &[u8],
        options: RasterizerOptions,
        page_gap: u32,
    ) -> Result<RenderedPage> {
        validate_pdf_bytes(bytes)?;
        self.render_document_range_from_bytes(bytes, 0, MAX_COMPOSITE_PAGES, options, page_gap)
    }

    /// Extracts text from a specific page (0-indexed) or the entire document if `page_index` is None.
    pub fn extract_text_from_bytes(
        &self,
        bytes: &[u8],
        page_index: Option<usize>,
    ) -> Result<String> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            text::extract_document_text(pdfium, bytes, page_index)
        } else if self.allow_mock_fallback {
            Ok(String::new())
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }

    /// Extracts full document details including text, text segments with bounding boxes, and links.
    pub fn extract_document_details(&self, bytes: &[u8]) -> Result<PdfDocumentDetails> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            let doc = pdfium
                .load_pdf_from_byte_slice(bytes, None)
                .context("Failed to load PDF for metadata extraction")?;

            let mut pages_details = Vec::new();
            let mut full_text = String::new();

            for (idx, page) in doc.pages().iter().enumerate() {
                let page_w = page.width().value;
                let page_h = page.height().value;

                let (page_text, segments, heuristic_links) =
                    text::extract_page_text_and_segments(&page, page_w, page_h);

                if !page_text.is_empty() {
                    if idx > 0 {
                        full_text.push_str("\n\n--- Page ");
                        full_text.push_str(&(idx + 1).to_string());
                        full_text.push_str(" ---\n\n");
                    }
                    full_text.push_str(&page_text);
                }

                let mut links = links::extract_page_links(&page, page_w, page_h);
                for h_link in heuristic_links {
                    let exists = links
                        .iter()
                        .any(|l| (l.x - h_link.x).abs() < 0.02 && (l.y - h_link.y).abs() < 0.02);
                    if !exists {
                        links.push(h_link);
                    }
                }

                pages_details.push(PdfPageDetails {
                    page_index: idx,
                    width_pt: page_w,
                    height_pt: page_h,
                    text: page_text,
                    links,
                    text_segments: segments,
                });
            }

            Ok(PdfDocumentDetails {
                total_pages: pages_details.len(),
                full_text,
                pages: pages_details,
            })
        } else if self.allow_mock_fallback {
            Ok(PdfDocumentDetails::default())
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }

    /// Renders all pages and extracts text/links in a single fast pass over the loaded document.
    pub fn render_and_extract_document_from_bytes(
        &self,
        bytes: &[u8],
        options: RasterizerOptions,
    ) -> Result<PdfDocumentRenderResult> {
        validate_pdf_bytes(bytes)?;
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            render::render_and_extract_document(pdfium, bytes, options)
        } else if self.allow_mock_fallback {
            let doc = PdfDocument::from_bytes(bytes, None)?;
            let total_pages = doc.total_pages();
            if total_pages > MAX_COMPOSITE_PAGES {
                anyhow::bail!(
                    "Document contains {} pages, exceeding maximum batch render budget of {} pages. Use per-page rendering.",
                    total_pages,
                    MAX_COMPOSITE_PAGES
                );
            }
            let dim = PageDimensions::new(612.0, 792.0);
            let page = crate::rasterizer::PageRasterizer::render_mock_page(0, dim, options)?;
            Ok(PdfDocumentRenderResult {
                total_pages: 1,
                full_text: String::new(),
                pages: vec![PdfPageRenderResult {
                    page_index: 0,
                    width: page.width,
                    height: page.height,
                    rgba_buffer: page.rgba_buffer.as_ref().clone(),
                    text: String::new(),
                    text_segments: Vec::new(),
                    links: Vec::new(),
                }],
            })
        } else {
            anyhow::bail!(
                "Native Pdfium engine is unavailable. Please install libpdfium or set PDFIUM_LIB_PATH."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_initialization_safety() {
        let engine = PdfiumEngine::new();
        let _ = engine.is_native_available();
    }

    #[test]
    fn test_native_absent_returns_explicit_err_without_fallback() {
        // Engine with no native library and mock fallback disabled (default production mode)
        let engine = PdfiumEngine {
            inner: Arc::new(Mutex::new(SerializedPdfium(None))),
            allow_mock_fallback: false,
        };

        let dummy_pdf = b"%PDF-1.7\nSample";
        let opts = RasterizerOptions::default();

        assert!(engine.load_document_from_bytes(dummy_pdf, None).is_err());
        assert!(engine.render_page_from_bytes(dummy_pdf, 0, opts).is_err());
        assert!(engine
            .render_document_range_from_bytes(dummy_pdf, 0, 1, opts, 10)
            .is_err());
        assert!(engine.extract_text_from_bytes(dummy_pdf, None).is_err());
        assert!(engine.extract_document_details(dummy_pdf).is_err());
        assert!(engine
            .render_and_extract_document_from_bytes(dummy_pdf, opts)
            .is_err());
    }

    #[test]
    fn test_fallback_rendering_when_explicitly_allowed() {
        let engine = PdfiumEngine::mock();

        let dummy_pdf = b"%PDF-1.7\nSample";
        let doc = engine
            .load_document_from_bytes(dummy_pdf, None)
            .expect("Fallback parse failed");
        assert_eq!(doc.total_pages(), 1);

        let opts = RasterizerOptions::default();
        let page = engine
            .render_page_from_bytes(dummy_pdf, 0, opts)
            .expect("Fallback render failed");
        assert!(page.width > 0);
        assert!(page.height > 0);
        assert_eq!(
            page.rgba_buffer.len(),
            (page.width * page.height * 4) as usize
        );
    }

    #[test]
    fn test_render_document_multi_page() {
        let engine = PdfiumEngine::mock();

        let dummy_pdf = b"%PDF-1.7\nSample";
        let opts = RasterizerOptions::default();
        let rendered = engine
            .render_document_from_bytes(dummy_pdf, opts, 20)
            .expect("Render document failed");
        assert!(rendered.width > 0);
        assert!(rendered.height > 0);
        assert_eq!(
            rendered.rgba_buffer.len(),
            (rendered.width * rendered.height * 4) as usize
        );
    }

    #[test]
    fn test_extract_document_details() {
        let engine = PdfiumEngine::mock();
        let dummy_pdf = b"%PDF-1.7\nSample";
        let details = engine.extract_document_details(dummy_pdf);
        assert!(details.is_ok());
        let doc = details.unwrap();
        assert_eq!(doc.total_pages, 0);
    }

    #[test]
    fn test_render_and_extract_document_from_bytes() {
        let engine = PdfiumEngine::mock();
        let dummy_pdf = b"%PDF-1.7\nSample";
        let opts = RasterizerOptions::default();
        let res = engine.render_and_extract_document_from_bytes(dummy_pdf, opts);
        assert!(res.is_ok());
        let doc = res.unwrap();
        assert_eq!(doc.total_pages, 1);
        assert_eq!(doc.pages.len(), 1);
        assert!(doc.pages[0].width > 0);
        assert!(doc.pages[0].height > 0);
    }

    #[test]
    fn test_max_page_dimension_bounds() {
        assert_eq!(MAX_PAGE_DIMENSION, 8192.0);
        let huge_dim: f32 = 100_000.0;
        let clamped = huge_dim.clamp(1.0, MAX_PAGE_DIMENSION);
        assert_eq!(clamped, 8192.0);

        let tiny_dim: f32 = -50.0;
        let clamped_tiny = tiny_dim.clamp(1.0, MAX_PAGE_DIMENSION);
        assert_eq!(clamped_tiny, 1.0);
    }

    #[test]
    fn test_magic_bytes_validation_rejection() {
        let engine = PdfiumEngine::mock();

        let invalid_pdf = b"NOT_A_VALID_PDF_BYTES";
        let opts = RasterizerOptions::default();

        assert!(validate_pdf_bytes(invalid_pdf).is_err());
        assert!(engine.load_document_from_bytes(invalid_pdf, None).is_err());
        assert!(engine.render_page_from_bytes(invalid_pdf, 0, opts).is_err());
        assert!(engine
            .render_document_from_bytes(invalid_pdf, opts, 10)
            .is_err());
        assert!(engine
            .render_document_range_from_bytes(invalid_pdf, 0, 5, opts, 10)
            .is_err());
        assert!(engine.extract_text_from_bytes(invalid_pdf, None).is_err());
        assert!(engine.extract_document_details(invalid_pdf).is_err());
        assert!(engine
            .render_and_extract_document_from_bytes(invalid_pdf, opts)
            .is_err());
    }

    #[test]
    fn test_composite_page_budget_caps() {
        let engine = PdfiumEngine::mock();

        let dummy_pdf = b"%PDF-1.7\nSample";
        let opts = RasterizerOptions::default();

        // Budget of 0 must fail immediately
        assert!(engine
            .render_document_range_from_bytes(dummy_pdf, 0, 0, opts, 10)
            .is_err());

        // Normal bounded render within budget must succeed
        assert!(engine
            .render_document_range_from_bytes(dummy_pdf, 0, 1, opts, 10)
            .is_ok());
    }
}
