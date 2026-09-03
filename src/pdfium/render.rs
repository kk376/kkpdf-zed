//! High-performance PDF page rasterization and composite viewport rendering.

use anyhow::{Context as _, Result};
use pdfium_render::prelude::*;

use crate::cache::RenderedPage;
use crate::document::{PageDimensions, PdfDocument};
use crate::rasterizer::{LuminosityToneMapper, PageRasterizer, RasterizerOptions};

use super::links::{extract_page_links, PdfLinkAnnotation};
use super::text::{extract_page_text_and_segments, PdfTextSegment};

/// Maximum dimension (width or height) allowed when rasterizing a PDF page,
/// preventing out-of-memory denial of service attacks from malicious documents.
pub const MAX_PAGE_DIMENSION: f32 = 8192.0;

/// Maximum number of pages permitted for continuous document stitching.
/// Large documents must use viewport-bounded tile rendering or per-page rendering to avoid multi-GB OOM.
pub const MAX_COMPOSITE_PAGES: usize = 16;

/// Maximum composite bitmap height in pixels to avoid massive memory allocations (32K px).
pub const MAX_COMPOSITE_HEIGHT: u32 = 32_768;

/// Maximum buffer size for composite document rendering (64 MB).
pub const MAX_COMPOSITE_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Result of rendering a single page alongside extracted text and hyperlinks.
#[derive(Debug, Clone)]
pub struct PdfPageRenderResult {
    pub page_index: usize,
    pub width: u32,
    pub height: u32,
    pub rgba_buffer: Vec<u8>,
    pub text: String,
    pub text_segments: Vec<PdfTextSegment>,
    pub links: Vec<PdfLinkAnnotation>,
}

/// Result of batch rendering and extracting an entire PDF document.
#[derive(Debug, Clone, Default)]
pub struct PdfDocumentRenderResult {
    pub total_pages: usize,
    pub full_text: String,
    pub pages: Vec<PdfPageRenderResult>,
}

/// Rasterizes a specific page from file bytes into an RGBA frame buffer using native Pdfium.
pub fn render_single_page(
    pdfium: &Pdfium,
    bytes: &[u8],
    page_index: usize,
    options: RasterizerOptions,
) -> Result<RenderedPage> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .context("Failed to load PDF in rasterizer")?;

    let page = doc
        .pages()
        .get(page_index as u16)
        .context("Requested page index out of bounds")?;

    let target_width = (page.width().value * (options.target_dpi / 72.0) * options.zoom_factor)
        .round()
        .clamp(1.0, MAX_PAGE_DIMENSION) as i32;
    let target_height = (page.height().value * (options.target_dpi / 72.0) * options.zoom_factor)
        .round()
        .clamp(1.0, MAX_PAGE_DIMENSION) as i32;

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
}

/// Rasterizes a bounded viewport range of pages from file bytes into a continuous vertical layout using native Pdfium.
pub fn render_document_range(
    pdfium: &Pdfium,
    bytes: &[u8],
    start_page: usize,
    page_budget: usize,
    options: RasterizerOptions,
    page_gap: u32,
) -> Result<RenderedPage> {
    if page_budget == 0 {
        anyhow::bail!("Page budget must be greater than zero");
    }
    let max_pages = page_budget.min(MAX_COMPOSITE_PAGES);

    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .context("Failed to load PDF in document rasterizer")?;

    let total_pages = doc.pages().len() as usize;
    if total_pages == 0 {
        anyhow::bail!("PDF document contains no pages");
    }
    if start_page >= total_pages {
        anyhow::bail!(
            "Start page index {} out of bounds (total: {})",
            start_page,
            total_pages
        );
    }

    let end_page = (start_page + max_pages).min(total_pages);
    let count = end_page - start_page;

    let mut rendered_pages = Vec::with_capacity(count);
    for idx in start_page..end_page {
        let page = doc
            .pages()
            .get(idx as u16)
            .context("Requested page index out of bounds")?;

        let target_width = (page.width().value * (options.target_dpi / 72.0) * options.zoom_factor)
            .round()
            .clamp(1.0, MAX_PAGE_DIMENSION) as i32;
        let target_height =
            (page.height().value * (options.target_dpi / 72.0) * options.zoom_factor)
                .round()
                .clamp(1.0, MAX_PAGE_DIMENSION) as i32;

        let render_config = PdfRenderConfig::new()
            .set_target_width(target_width)
            .set_target_height(target_height)
            .render_form_data(true)
            .render_annotations(true);

        let bitmap = page
            .render_with_config(&render_config)
            .with_context(|| format!("Pdfium failed to render page {}", idx))?;

        let mut rgba_buffer = bitmap.as_image().to_rgba8().into_raw();

        if options.dark_mode {
            LuminosityToneMapper::apply(&mut rgba_buffer, options.saturation_threshold);
        }

        rendered_pages.push((target_width as u32, target_height as u32, rgba_buffer));
    }

    composite_pages(start_page, rendered_pages, options, page_gap, count)
}

/// Renders all pages and extracts text/links in a single pass using native Pdfium.
pub fn render_and_extract_document(
    pdfium: &Pdfium,
    bytes: &[u8],
    options: RasterizerOptions,
) -> Result<PdfDocumentRenderResult> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .context("Failed to load PDF for render and extraction")?;

    let total_pages = doc.pages().len() as usize;
    if total_pages > MAX_COMPOSITE_PAGES {
        anyhow::bail!(
            "Document contains {} pages, exceeding maximum batch render budget of {} pages. Use per-page rendering.",
            total_pages,
            MAX_COMPOSITE_PAGES
        );
    }
    let mut full_text = String::new();
    let mut rendered_pages = Vec::with_capacity(total_pages);

    for (idx, page) in doc.pages().iter().enumerate() {
        let page_w = page.width().value;
        let page_h = page.height().value;

        let target_width = (page_w * (options.target_dpi / 72.0) * options.zoom_factor)
            .round()
            .clamp(1.0, MAX_PAGE_DIMENSION) as i32;
        let target_height = (page_h * (options.target_dpi / 72.0) * options.zoom_factor)
            .round()
            .clamp(1.0, MAX_PAGE_DIMENSION) as i32;

        let render_config = PdfRenderConfig::new()
            .set_target_width(target_width)
            .set_target_height(target_height)
            .render_form_data(true)
            .render_annotations(true);

        let bitmap = page
            .render_with_config(&render_config)
            .with_context(|| format!("Pdfium failed to render page {}", idx))?;

        let mut rgba_buffer = bitmap.as_image().to_rgba8().into_raw();

        if options.dark_mode {
            LuminosityToneMapper::apply(&mut rgba_buffer, options.saturation_threshold);
        }

        let (page_text, text_segments, heuristic_links) =
            extract_page_text_and_segments(&page, page_w, page_h);

        if !page_text.is_empty() {
            if idx > 0 {
                full_text.push_str("\n\n--- Page ");
                full_text.push_str(&(idx + 1).to_string());
                full_text.push_str(" ---\n\n");
            }
            full_text.push_str(&page_text);
        }

        let mut links = extract_page_links(&page, page_w, page_h);
        for h_link in heuristic_links {
            let exists = links
                .iter()
                .any(|l| (l.x - h_link.x).abs() < 0.02 && (l.y - h_link.y).abs() < 0.02);
            if !exists {
                links.push(h_link);
            }
        }

        rendered_pages.push(PdfPageRenderResult {
            page_index: idx,
            width: target_width as u32,
            height: target_height as u32,
            rgba_buffer,
            text: page_text,
            text_segments,
            links,
        });
    }

    Ok(PdfDocumentRenderResult {
        total_pages: rendered_pages.len(),
        full_text,
        pages: rendered_pages,
    })
}

// -----------------------------------------------------------------------------
// Synthetic Mock Fallbacks (Strictly for test harnesses)
// -----------------------------------------------------------------------------

/// Renders a synthetic mock page for test harnesses when native Pdfium is not available.
pub fn render_mock_single_page(
    page_index: usize,
    options: RasterizerOptions,
) -> Result<RenderedPage> {
    let dim = PageDimensions::new(612.0, 792.0);
    PageRasterizer::render_mock_page(page_index, dim, options)
}

/// Renders a composite document range using synthetic mock pages for test harnesses.
pub fn render_mock_document_range(
    bytes: &[u8],
    start_page: usize,
    page_budget: usize,
    options: RasterizerOptions,
    page_gap: u32,
) -> Result<RenderedPage> {
    if page_budget == 0 {
        anyhow::bail!("Page budget must be greater than zero");
    }
    let max_pages = page_budget.min(MAX_COMPOSITE_PAGES);

    let doc = PdfDocument::from_bytes(bytes, None)?;
    let total_pages = doc.total_pages();
    if total_pages == 0 {
        anyhow::bail!("PDF document contains no pages");
    }
    if start_page >= total_pages {
        anyhow::bail!(
            "Start page index {} out of bounds (total: {})",
            start_page,
            total_pages
        );
    }

    let end_page = (start_page + max_pages).min(total_pages);
    let count = end_page - start_page;

    if count == 1 {
        let dim = doc
            .page_size(start_page)
            .unwrap_or(PageDimensions::new(612.0, 792.0));
        return PageRasterizer::render_mock_page(start_page, dim, options);
    }

    let mut rendered_pages = Vec::with_capacity(count);
    for i in start_page..end_page {
        let dim = doc
            .page_size(i)
            .unwrap_or(PageDimensions::new(612.0, 792.0));
        let page = PageRasterizer::render_mock_page(i, dim, options)?;
        rendered_pages.push((page.width, page.height, page.rgba_buffer.as_ref().clone()));
    }

    composite_pages(start_page, rendered_pages, options, page_gap, count)
}

/// Blits a vector of rendered page bitmaps into a single continuous vertical composite.
fn composite_pages(
    start_page: usize,
    rendered_pages: Vec<(u32, u32, Vec<u8>)>,
    options: RasterizerOptions,
    page_gap: u32,
    count: usize,
) -> Result<RenderedPage> {
    let max_width = rendered_pages.iter().map(|(w, _, _)| *w).max().unwrap_or(1);
    let total_height: u32 =
        rendered_pages.iter().map(|(_, h, _)| *h).sum::<u32>() + ((count as u32 - 1) * page_gap);

    if total_height > MAX_COMPOSITE_HEIGHT {
        anyhow::bail!(
            "Total composite height ({} px) exceeds maximum limit ({} px)",
            total_height,
            MAX_COMPOSITE_HEIGHT
        );
    }

    let stride = (max_width as usize) * 4;
    let total_bytes = stride * (total_height as usize);

    if total_bytes > MAX_COMPOSITE_BUFFER_BYTES {
        anyhow::bail!(
            "Composite document buffer size ({} bytes) exceeds safety budget ({} bytes). Use per-page rendering.",
            total_bytes,
            MAX_COMPOSITE_BUFFER_BYTES
        );
    }

    let bg_color = if options.dark_mode {
        [24u8, 24, 37, 255]
    } else {
        [230u8, 233, 239, 255]
    };

    let mut composite = vec![0u8; total_bytes];
    for chunk in composite.chunks_exact_mut(4) {
        chunk.copy_from_slice(&bg_color);
    }

    let mut y_offset = 0u32;
    for (w, h, page_buf) in &rendered_pages {
        let x_offset = ((max_width - w) / 2) as usize;
        let page_stride = (*w as usize) * 4;

        for y in 0..*h {
            let src_start = (y as usize) * page_stride;
            let src_end = src_start + page_stride;
            let src_slice = &page_buf[src_start..src_end];

            let dst_y = (y_offset + y) as usize;
            let dst_start = (dst_y * (max_width as usize) + x_offset) * 4;
            let dst_end = dst_start + page_stride;

            composite[dst_start..dst_end].copy_from_slice(src_slice);
        }

        y_offset += h + page_gap;
    }

    Ok(RenderedPage::new(
        start_page,
        max_width,
        total_height,
        options.zoom_factor,
        options.dark_mode,
        composite,
    ))
}
