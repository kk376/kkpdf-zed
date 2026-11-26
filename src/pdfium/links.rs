//! PDF hyperlink and annotation extraction.

use pdfium_render::prelude::*;

/// Hyperlink annotation with normalized bounding box coordinates [0.0, 1.0].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PdfLinkAnnotation {
    pub url: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Extracts link annotations from a PdfPage, including URI actions and link annotations.
pub fn extract_page_links(page: &PdfPage, page_w: f32, page_h: f32) -> Vec<PdfLinkAnnotation> {
    let mut links = Vec::new();

    // 1. Links from page.links()
    for link in page.links().iter() {
        if let Some(PdfAction::Uri(uri_action)) = link.action()
            && let Ok(url) = uri_action.uri()
        {
            let url_clean = url.trim().to_string();
            if !url_clean.is_empty()
                && let Ok(rect) = link.rect()
            {
                let (norm_x, norm_y, norm_w, norm_h) = normalize_rect(rect, page_w, page_h);
                if norm_w > 0.0 && norm_h > 0.0 {
                    links.push(PdfLinkAnnotation {
                        url: url_clean,
                        x: norm_x,
                        y: norm_y,
                        width: norm_w,
                        height: norm_h,
                    });
                }
            }
        }
    }

    // 2. Links from page.annotations()
    for annot in page.annotations().iter() {
        if let Some(link_annot) = annot.as_link_annotation()
            && let Ok(link) = link_annot.link()
            && let Some(PdfAction::Uri(uri_action)) = link.action()
            && let Ok(url) = uri_action.uri()
        {
            let url_clean = url.trim().to_string();
            if !url_clean.is_empty()
                && let Ok(rect) = link.rect()
            {
                let (norm_x, norm_y, norm_w, norm_h) = normalize_rect(rect, page_w, page_h);
                if norm_w > 0.0 && norm_h > 0.0 {
                    let exists = links.iter().any(|l| {
                        l.url == url_clean
                            && (l.x - norm_x).abs() < 0.01
                            && (l.y - norm_y).abs() < 0.01
                    });
                    if !exists {
                        links.push(PdfLinkAnnotation {
                            url: url_clean,
                            x: norm_x,
                            y: norm_y,
                            width: norm_w,
                            height: norm_h,
                        });
                    }
                }
            }
        }
    }

    links
}

/// Normalizes PDF point coordinates (bottom-left origin) to top-left screen coordinates `[0.0, 1.0]`.
pub fn normalize_coords(
    raw_left: f32,
    raw_top: f32,
    raw_right: f32,
    raw_bottom: f32,
    page_w: f32,
    page_h: f32,
) -> (f32, f32, f32, f32) {
    let left = raw_left.min(raw_right);
    let right = raw_left.max(raw_right);
    let bottom = raw_bottom.min(raw_top);
    let top = raw_bottom.max(raw_top);

    let (norm_x, norm_w) = if page_w.is_finite() && page_w > 0.0 {
        (
            (left / page_w).clamp(0.0, 1.0),
            ((right - left) / page_w).clamp(0.0, 1.0),
        )
    } else {
        (0.0, 0.0)
    };
    let (norm_y, norm_h) = if page_h.is_finite() && page_h > 0.0 {
        (
            ((page_h - top) / page_h).clamp(0.0, 1.0),
            ((top - bottom) / page_h).clamp(0.0, 1.0),
        )
    } else {
        (0.0, 0.0)
    };

    (norm_x, norm_y, norm_w, norm_h)
}

/// Normalizes PDF points (bottom-left origin) to screen percentage coordinates `[0.0, 1.0]` (top-left origin).
pub fn normalize_rect(
    rect: pdfium_render::prelude::PdfRect,
    page_w: f32,
    page_h: f32,
) -> (f32, f32, f32, f32) {
    normalize_coords(
        rect.left().value,
        rect.top().value,
        rect.right().value,
        rect.bottom().value,
        page_w,
        page_h,
    )
}
