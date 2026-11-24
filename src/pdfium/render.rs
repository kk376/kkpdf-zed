//! High-performance PDF page rasterization and composite viewport rendering.

use anyhow::{Context as _, Result};
use pdfium_render::prelude::*;

use crate::cache::RenderedPage;
use crate::document::{PageDimensions, PdfDocument};
use crate::rasterizer::{LuminosityToneMapper, PageRasterizer, RasterizerOptions};

use super::links::{PdfLinkAnnotation, extract_page_links};
use super::text::{PdfTextSegment, extract_page_text_and_segments};

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

/// Computes aspect-preserving target bitmap dimensions capped at `MAX_PAGE_DIMENSION`.
fn scaled_page_dimensions(page_w: f32, page_h: f32, options: RasterizerOptions) -> (u32, u32) {
    let scale = if options.target_dpi.is_finite()
        && options.zoom_factor.is_finite()
        && options.target_dpi > 0.0
        && options.zoom_factor > 0.0
    {
        (options.target_dpi / 72.0) * options.zoom_factor
    } else {
        1.0
    };
    let safe_w = if page_w.is_finite() && page_w > 0.0 {
        page_w
    } else {
        1.0
    };
    let safe_h = if page_h.is_finite() && page_h > 0.0 {
        page_h
    } else {
        1.0
    };

    let mut w = (safe_w * scale).max(1.0);
    let mut h = (safe_h * scale).max(1.0);
    let max_side = w.max(h);
    if max_side > MAX_PAGE_DIMENSION {
        let factor = MAX_PAGE_DIMENSION / max_side;
        w *= factor;
        h *= factor;
    }

    (w.round().max(1.0) as u32, h.round().max(1.0) as u32)
}

/// Validates that compositing `sizes` vertically with `page_gap` stays within height and byte budgets.
fn validate_composite_budget(sizes: &[(u32, u32)], page_gap: u32) -> Result<(u32, u32, usize)> {
    if sizes.is_empty() {
        anyhow::bail!("Cannot composite zero pages");
    }

    let max_width = sizes.iter().map(|(w, _)| *w).max().unwrap_or(1).max(1);
    let pages_height = sizes
        .iter()
        .try_fold(0u32, |acc, (_, h)| acc.checked_add(*h));
    let gaps_height = u32::try_from(sizes.len() - 1)
        .ok()
        .and_then(|gaps| gaps.checked_mul(page_gap));
    let total_height = pages_height
        .and_then(|ph| gaps_height.and_then(|gh| ph.checked_add(gh)))
        .ok_or_else(|| anyhow::anyhow!("Composite bitmap height overflow"))?;

    if total_height > MAX_COMPOSITE_HEIGHT {
        anyhow::bail!(
            "Total composite height ({} px) exceeds maximum limit ({} px)",
            total_height,
            MAX_COMPOSITE_HEIGHT
        );
    }

    let total_bytes = (max_width as usize)
        .checked_mul(total_height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("Composite bitmap buffer size overflow"))?;

    if total_bytes > MAX_COMPOSITE_BUFFER_BYTES {
        anyhow::bail!(
            "Composite document buffer size ({} bytes) exceeds safety budget ({} bytes). Use per-page rendering.",
            total_bytes,
            MAX_COMPOSITE_BUFFER_BYTES
        );
    }

    Ok((max_width, total_height, total_bytes))
}

/// Rasterizes a single Pdfium page to an un-tonemapped tightly packed RGBA buffer.
fn rasterize_pdf_page_rgba(
    page: &PdfPage,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>> {
    let w_i32 = i32::try_from(target_width).context("Page target width exceeds i32")?;
    let h_i32 = i32::try_from(target_height).context("Page target height exceeds i32")?;

    let render_config = PdfRenderConfig::new()
        .set_target_width(w_i32)
        .set_target_height(h_i32)
        .render_form_data(true)
        .render_annotations(true);

    let bitmap = page
        .render_with_config(&render_config)
        .context("Pdfium page render call failed")?;

    let rgba_buffer = bitmap.as_rgba_bytes();
    let expected = (target_width as usize)
        .checked_mul(target_height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("Rendered page byte size overflow"))?;
    if rgba_buffer.len() != expected {
        anyhow::bail!(
            "Pdfium bitmap buffer length mismatch: expected {} bytes, got {}",
            expected,
            rgba_buffer.len()
        );
    }
    Ok(rgba_buffer)
}

/// Rasterizes a page into raw RGBA pixels using Pdfium without running tone mapping.
pub(crate) fn render_single_page_raw(
    pdfium: &Pdfium,
    bytes: &[u8],
    page_index: usize,
    options: RasterizerOptions,
) -> Result<(u32, u32, Vec<u8>)> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .context("Failed to load PDF in rasterizer")?;

    let page_idx = page_index
        .try_into()
        .map_err(|_| anyhow::anyhow!("Requested page index {} exceeds Pdfium range", page_index))?;
    let page = doc
        .pages()
        .get(page_idx)
        .context("Requested page index out of bounds")?;

    let (target_width, target_height) =
        scaled_page_dimensions(page.width().value, page.height().value, options);
    let rgba_buffer = rasterize_pdf_page_rgba(&page, target_width, target_height)?;
    Ok((target_width, target_height, rgba_buffer))
}

/// Finalizes a raw RGBA buffer (optionally dark-mode mapped) into a `RenderedPage`.
pub(crate) fn finalize_rendered_page(
    page_index: usize,
    width: u32,
    height: u32,
    mut rgba_buffer: Vec<u8>,
    options: RasterizerOptions,
) -> RenderedPage {
    if options.dark_mode {
        LuminosityToneMapper::apply(&mut rgba_buffer, options.saturation_threshold);
    }
    RenderedPage::new(
        page_index,
        width,
        height,
        options.zoom_factor,
        options.dark_mode,
        rgba_buffer,
    )
}

/// Rasterizes a specific page from file bytes into an RGBA frame buffer using native Pdfium.
pub fn render_single_page(
    pdfium: &Pdfium,
    bytes: &[u8],
    page_index: usize,
    options: RasterizerOptions,
) -> Result<RenderedPage> {
    let (w, h, rgba) = render_single_page_raw(pdfium, bytes, page_index, options)?;
    Ok(finalize_rendered_page(page_index, w, h, rgba, options))
}

/// Rasterizes raw RGBA bitmaps for a budgeted range after pre-checking composite budgets.
pub(crate) fn render_document_range_raw(
    pdfium: &Pdfium,
    bytes: &[u8],
    start_page: usize,
    page_budget: usize,
    options: RasterizerOptions,
    page_gap: u32,
) -> Result<Vec<(u32, u32, Vec<u8>)>> {
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

    let end_page = start_page.saturating_add(max_pages).min(total_pages);
    let count = end_page - start_page;

    let mut projected_sizes = Vec::with_capacity(count);
    for idx in start_page..end_page {
        let page_idx = idx
            .try_into()
            .map_err(|_| anyhow::anyhow!("Requested page index {} exceeds Pdfium range", idx))?;
        let page = doc
            .pages()
            .get(page_idx)
            .context("Requested page index out of bounds")?;
        projected_sizes.push(scaled_page_dimensions(
            page.width().value,
            page.height().value,
            options,
        ));
    }

    // Check composite height and byte limits BEFORE allocating page framebuffers.
    validate_composite_budget(&projected_sizes, page_gap)?;

    let mut rendered_pages = Vec::with_capacity(count);
    for (offset, (target_width, target_height)) in projected_sizes.into_iter().enumerate() {
        let idx = start_page + offset;
        let page_idx = idx
            .try_into()
            .map_err(|_| anyhow::anyhow!("Requested page index {} exceeds Pdfium range", idx))?;
        let page = doc
            .pages()
            .get(page_idx)
            .context("Requested page index out of bounds")?;
        let rgba_buffer = rasterize_pdf_page_rgba(&page, target_width, target_height)
            .with_context(|| format!("Pdfium failed to render page {}", idx))?;
        rendered_pages.push((target_width, target_height, rgba_buffer));
    }

    Ok(rendered_pages)
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
    let rendered_pages =
        render_document_range_raw(pdfium, bytes, start_page, page_budget, options, page_gap)?;
    finalize_composite_pages(start_page, rendered_pages, options, page_gap)
}

/// Applies tone mapping (when enabled) and composites rendered pages outside any Pdfium lock.
pub(crate) fn finalize_composite_pages(
    start_page: usize,
    mut rendered_pages: Vec<(u32, u32, Vec<u8>)>,
    options: RasterizerOptions,
    page_gap: u32,
) -> Result<RenderedPage> {
    if options.dark_mode {
        for (_, _, buf) in &mut rendered_pages {
            LuminosityToneMapper::apply(buf, options.saturation_threshold);
        }
    }
    composite_pages(start_page, rendered_pages, options, page_gap)
}

/// Renders all pages and extracts text/links in a single pass using native Pdfium without tone mapping.
pub(crate) fn render_and_extract_document_raw(
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

    let mut page_dims = Vec::with_capacity(total_pages);
    let mut cumulative_bytes = 0usize;
    for page in doc.pages().iter() {
        let page_w = page.width().value;
        let page_h = page.height().value;
        let (target_w, target_h) = scaled_page_dimensions(page_w, page_h, options);
        let page_bytes = (target_w as usize)
            .checked_mul(target_h as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("Batch render page byte size overflow"))?;
        cumulative_bytes = cumulative_bytes
            .checked_add(page_bytes)
            .ok_or_else(|| anyhow::anyhow!("Batch render total buffer size overflow"))?;
        if cumulative_bytes > MAX_COMPOSITE_BUFFER_BYTES {
            anyhow::bail!(
                "Batch document buffer size exceeds safety budget ({} bytes). Use per-page rendering.",
                MAX_COMPOSITE_BUFFER_BYTES
            );
        }
        page_dims.push((page_w, page_h, target_w, target_h));
    }

    let mut full_text = String::new();
    let mut rendered_pages = Vec::with_capacity(total_pages);

    for (idx, page) in doc.pages().iter().enumerate() {
        let (page_w, page_h, target_width, target_height) = page_dims[idx];
        let rgba_buffer = rasterize_pdf_page_rgba(&page, target_width, target_height)
            .with_context(|| format!("Pdfium failed to render page {}", idx))?;

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
            width: target_width,
            height: target_height,
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

/// Applies dark-mode tone mapping across a batch extraction result outside any Pdfium lock.
pub(crate) fn finalize_document_extract_result(
    mut res: PdfDocumentRenderResult,
    options: RasterizerOptions,
) -> PdfDocumentRenderResult {
    if options.dark_mode {
        for page in &mut res.pages {
            LuminosityToneMapper::apply(&mut page.rgba_buffer, options.saturation_threshold);
        }
    }
    res
}

/// Renders all pages and extracts text/links in a single pass using native Pdfium.
pub fn render_and_extract_document(
    pdfium: &Pdfium,
    bytes: &[u8],
    options: RasterizerOptions,
) -> Result<PdfDocumentRenderResult> {
    let raw = render_and_extract_document_raw(pdfium, bytes, options)?;
    Ok(finalize_document_extract_result(raw, options))
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

    composite_pages(start_page, rendered_pages, options, page_gap)
}

/// Blits a vector of rendered page bitmaps into a single continuous vertical composite.
fn composite_pages(
    start_page: usize,
    rendered_pages: Vec<(u32, u32, Vec<u8>)>,
    options: RasterizerOptions,
    page_gap: u32,
) -> Result<RenderedPage> {
    let sizes: Vec<(u32, u32)> = rendered_pages.iter().map(|(w, h, _)| (*w, *h)).collect();
    let (max_width, total_height, total_bytes) = validate_composite_budget(&sizes, page_gap)?;

    let bg_color = if options.dark_mode {
        [24u8, 24, 37, 255]
    } else {
        [230u8, 233, 239, 255]
    };

    let mut composite = vec![0u8; total_bytes];
    for chunk in composite.as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&bg_color);
    }

    let mut y_offset = 0u32;
    for (w, h, page_buf) in &rendered_pages {
        let x_offset = ((max_width - w) / 2) as usize;
        let page_stride = (*w as usize)
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("Page stride overflow"))?;

        for y in 0..*h {
            let src_start = (y as usize) * page_stride;
            let src_end = src_start + page_stride;
            let src_slice = &page_buf[src_start..src_end];

            let dst_y = y_offset.saturating_add(y) as usize;
            let dst_start = dst_y
                .checked_mul(max_width as usize)
                .and_then(|row| row.checked_add(x_offset))
                .and_then(|px| px.checked_mul(4))
                .ok_or_else(|| anyhow::anyhow!("Composite blit offset overflow"))?;
            let dst_end = dst_start + page_stride;

            composite[dst_start..dst_end].copy_from_slice(src_slice);
        }

        y_offset = y_offset.saturating_add(*h).saturating_add(page_gap);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scaled_page_dimensions_preserves_aspect_ratio() {
        let opts = RasterizerOptions {
            target_dpi: 72.0,
            zoom_factor: 1.0,
            ..Default::default()
        };
        let (w, h) = scaled_page_dimensions(16_384.0, 2_048.0, opts);
        assert_eq!(w, 8192);
        assert_eq!(h, 1024);
    }

    #[test]
    fn test_composite_pages_empty_returns_err() {
        let res = composite_pages(0, Vec::new(), RasterizerOptions::default(), 16);
        assert!(res.is_err());
    }

    #[test]
    fn test_validate_composite_budget_rejects_oversized_before_alloc() {
        let oversized = vec![(4096u32, 4096u32); 4];
        assert!(validate_composite_budget(&oversized, 16).is_err());
    }
}
