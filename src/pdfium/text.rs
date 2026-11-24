//! PDF text extraction, word segmentation, and bounding box computation.

use anyhow::{Context as _, Result};
use pdfium_render::prelude::*;

use super::links::PdfLinkAnnotation;

/// Text segment with normalized screen coordinates [0.0, 1.0].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PdfTextSegment {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

fn push_normalized_segment(
    segments: &mut Vec<PdfTextSegment>,
    text: String,
    bounds: (f32, f32, f32, f32),
    page_dims: (f32, f32),
) {
    if text.is_empty() {
        return;
    }
    let (norm_x, norm_y, norm_w, norm_h) = super::links::normalize_coords(
        bounds.0,
        bounds.1,
        bounds.2,
        bounds.3,
        page_dims.0,
        page_dims.1,
    );
    if norm_w > 0.0 && norm_h > 0.0 {
        segments.push(PdfTextSegment {
            text,
            x: norm_x,
            y: norm_y,
            width: norm_w,
            height: norm_h,
        });
    }
}

/// Extracts text and segmented word bounding boxes from a single PdfPage.
pub fn extract_page_text_and_segments(
    page: &PdfPage,
    page_w: f32,
    page_h: f32,
) -> (String, Vec<PdfTextSegment>, Vec<PdfLinkAnnotation>) {
    let mut page_text = String::new();
    let mut segments = Vec::new();
    let mut heuristic_links = Vec::new();
    let page_dims = (page_w, page_h);

    if let Ok(text_page) = page.text() {
        page_text = text_page.all();

        let seg_coll = text_page.segments();
        for seg in seg_coll.iter() {
            let seg_str = seg.text();
            let seg_trimmed = seg_str.trim();
            if seg_trimmed.is_empty() {
                continue;
            }

            if !seg_trimmed.chars().any(char::is_whitespace) {
                let bounds = seg.bounds();
                push_normalized_segment(
                    &mut segments,
                    seg_trimmed.to_string(),
                    (
                        bounds.left().value,
                        bounds.top().value,
                        bounds.right().value,
                        bounds.bottom().value,
                    ),
                    page_dims,
                );
                continue;
            }

            if let Ok(chars) = seg.chars() {
                let mut word_text = String::new();
                let mut min_left = f32::MAX;
                let mut max_right = f32::MIN;
                let mut min_bottom = f32::MAX;
                let mut max_top = f32::MIN;
                let mut has_word_char = false;

                for ch in chars.iter() {
                    let u_char = ch.unicode_char();
                    let is_ws = u_char.map(|c| c.is_whitespace()).unwrap_or(false);

                    if is_ws {
                        if has_word_char && !word_text.is_empty() {
                            push_normalized_segment(
                                &mut segments,
                                std::mem::take(&mut word_text),
                                (min_left, max_top, max_right, min_bottom),
                                page_dims,
                            );
                            word_text.clear();
                            min_left = f32::MAX;
                            max_right = f32::MIN;
                            min_bottom = f32::MAX;
                            max_top = f32::MIN;
                            has_word_char = false;
                        }
                    } else if let Some(c) = u_char {
                        word_text.push(c);
                        if let Ok(bounds) = ch.loose_bounds().or_else(|_| ch.tight_bounds()) {
                            let l = bounds.left().value.min(bounds.right().value);
                            let r = bounds.left().value.max(bounds.right().value);
                            let b = bounds.bottom().value.min(bounds.top().value);
                            let t = bounds.bottom().value.max(bounds.top().value);

                            min_left = min_left.min(l);
                            max_right = max_right.max(r);
                            min_bottom = min_bottom.min(b);
                            max_top = max_top.max(t);
                            has_word_char = true;
                        }
                    }
                }

                if has_word_char && !word_text.is_empty() {
                    push_normalized_segment(
                        &mut segments,
                        word_text,
                        (min_left, max_top, max_right, min_bottom),
                        page_dims,
                    );
                }
            } else {
                let bounds = seg.bounds();
                push_normalized_segment(
                    &mut segments,
                    seg_trimmed.to_string(),
                    (
                        bounds.left().value,
                        bounds.top().value,
                        bounds.right().value,
                        bounds.bottom().value,
                    ),
                    page_dims,
                );
            }
        }
    }

    // Heuristic link discovery from plain text URLs
    for seg in &segments {
        let text = seg.text.trim();
        if text.starts_with("http://") || text.starts_with("https://") || text.starts_with("www.") {
            let url = if text.starts_with("www.") {
                format!("https://{}", text)
            } else {
                text.to_string()
            };
            heuristic_links.push(PdfLinkAnnotation {
                url,
                x: seg.x,
                y: seg.y,
                width: seg.width,
                height: seg.height,
            });
        }
    }

    (page_text, segments, heuristic_links)
}

/// Extracts text from a document or specific page using native Pdfium bindings.
pub fn extract_document_text(
    pdfium: &Pdfium,
    bytes: &[u8],
    page_index: Option<usize>,
) -> Result<String> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .context("Failed to load PDF for text extraction")?;

    if let Some(idx) = page_index {
        let page = doc
            .pages()
            .get(idx as i32)
            .context("Requested page index out of bounds")?;
        let text_page = page.text().context("Failed to load page text")?;
        Ok(text_page.all())
    } else {
        let mut full_text = String::new();
        for (idx, page) in doc.pages().iter().enumerate() {
            if let Ok(text_page) = page.text() {
                let page_text = text_page.all();
                if !page_text.is_empty() {
                    if idx > 0 {
                        full_text.push_str("\n\n--- Page ");
                        full_text.push_str(&(idx + 1).to_string());
                        full_text.push_str(" ---\n\n");
                    }
                    full_text.push_str(&page_text);
                }
            }
        }
        Ok(full_text)
    }
}
