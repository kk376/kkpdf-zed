//! High-level Pdfium engine abstraction and dynamic library loader.
//!
//! Provides thread-safe document rendering via Google Pdfium C++ engine,
//! with automated fallback and library path discovery.

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::RenderedPage;
use crate::document::{PageDimensions, PdfDocument};
use crate::rasterizer::{LuminosityToneMapper, RasterizerOptions};

/// Internal wrapper granting thread-safe synchronization to Pdfium handle.
struct SerializedPdfium(Option<Pdfium>);

// SAFETY: All calls to the internal Pdfium C++ FFI handle are strictly guarded
// by a Mutual Exclusion (Mutex) lock, guaranteeing serialized single-thread
// execution across asynchronous worker threads without race conditions.
unsafe impl Send for SerializedPdfium {}
unsafe impl Sync for SerializedPdfium {}

/// Thread-safe wrapper around a Pdfium instance.
#[derive(Clone)]
pub struct PdfiumEngine {
    inner: Arc<Mutex<SerializedPdfium>>,
}

impl Default for PdfiumEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfiumEngine {
    /// Creates a new engine handle, attempting dynamic binding initialization.
    pub fn new() -> Self {
        let pdfium = Self::try_init_pdfium();
        Self {
            inner: Arc::new(Mutex::new(SerializedPdfium(pdfium))),
        }
    }

    /// Attempts to bind to dynamic `libpdfium.so`, `libpdfium.dylib`, or `pdfium.dll`.
    fn try_init_pdfium() -> Option<Pdfium> {
        // 1. Try standard system library search path
        if let Ok(bindings) = Pdfium::bind_to_system_library() {
            log::info!("Successfully initialized Pdfium from system dynamic library");
            return Some(Pdfium::new(bindings));
        }

        // 2. Try common Linux/macOS shared object locations
        let common_paths = [
            "/usr/lib/libpdfium.so",
            "/usr/lib64/libpdfium.so",
            "/usr/local/lib/libpdfium.so",
            "/opt/homebrew/lib/libpdfium.dylib",
            "/usr/local/lib/libpdfium.dylib",
        ];

        for path_str in common_paths {
            let path = Path::new(path_str);
            if path.exists() {
                if let Ok(bindings) = Pdfium::bind_to_library(path) {
                    log::info!("Successfully bound to Pdfium at {path_str}");
                    return Some(Pdfium::new(bindings));
                }
            }
        }

        log::warn!("Dynamic libpdfium not found in standard system paths; engine will use synthetic fallback");
        None
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

    /// Loads a PDF from in-memory byte slice.
    pub fn load_document_from_bytes(
        &self,
        bytes: &[u8],
        path: Option<PathBuf>,
    ) -> Result<PdfDocument> {
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

            let title = doc.metadata().get(PdfDocumentMetadataTagType::Title).map(|s| s.value().to_string());

            Ok(PdfDocument::new(path, pages, title))
        } else {
            // Fallback scanner if native library is absent
            PdfDocument::from_bytes(bytes, path)
        }
    }

    /// Rasterizes a specific page from file bytes into an RGBA frame buffer.
    pub fn render_page_from_bytes(
        &self,
        bytes: &[u8],
        page_index: usize,
        options: RasterizerOptions,
    ) -> Result<RenderedPage> {
        let guard = self.inner.lock();
        if let Some(ref pdfium) = guard.0 {
            let doc = pdfium
                .load_pdf_from_byte_slice(bytes, None)
                .context("Failed to load PDF in rasterizer")?;

            let page = doc
                .pages()
                .get(page_index as u16)
                .context("Requested page index out of bounds")?;

            let target_width = (page.width().value * (options.target_dpi / 72.0) * options.zoom_factor)
                .round()
                .max(1.0) as i32;
            let target_height = (page.height().value * (options.target_dpi / 72.0) * options.zoom_factor)
                .round()
                .max(1.0) as i32;

            let render_config = PdfRenderConfig::new()
                .set_target_width(target_width)
                .set_target_height(target_height)
                .render_form_data(true)
                .render_annotations(true);

            let bitmap = page
                .render_with_config(&render_config)
                .context("Pdfium page render call failed")?;

            let mut rgba_buffer = bitmap.as_image().to_rgba8().into_raw();

            if options.dark_mode {
                LuminosityToneMapper::apply(&mut rgba_buffer, options.saturation_threshold);
            }

            Ok(RenderedPage::new(
                page_index,
                target_width as u32,
                target_height as u32,
                options.zoom_factor,
                options.dark_mode,
                rgba_buffer,
            ))
        } else {
            // Fallback mock rendering when native engine binary is not linked
            let dim = PageDimensions::new(612.0, 792.0);
            crate::rasterizer::PageRasterizer::render_mock_page(page_index, dim, options)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_initialization_safety() {
        let engine = PdfiumEngine::new();
        // Even without system libpdfium installed, instance must initialize safely
        let is_avail = engine.is_native_available();
        let _ = is_avail; // No panic
    }

    #[test]
    fn test_fallback_rendering_when_native_absent() {
        let engine = PdfiumEngine {
            inner: Arc::new(Mutex::new(SerializedPdfium(None))),
        };

        let dummy_pdf = b"%PDF-1.7\nSample";
        let doc = engine.load_document_from_bytes(dummy_pdf, None).expect("Fallback parse failed");
        assert_eq!(doc.total_pages(), 1);

        let opts = RasterizerOptions::default();
        let page = engine
            .render_page_from_bytes(dummy_pdf, 0, opts)
            .expect("Fallback render failed");
        assert!(page.width > 0);
        assert!(page.height > 0);
        assert_eq!(page.rgba_buffer.len(), (page.width * page.height * 4) as usize);
    }
}
