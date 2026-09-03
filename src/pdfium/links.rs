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
        if let Some(PdfAction::Uri(uri_action)) = link.action() {
            if let Ok(url) = uri_action.uri() {
                let url_clean = url.trim().to_string();
                if !url_clean.is_empty() {
                    if let Ok(rect) = link.rect() {
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
        }
    }

    // 2. Links from page.annotations()
    for annot in page.annotations().iter() {
        if let Some(link_annot) = annot.as_link_annotation() {
            if let Ok(link) = link_annot.link() {
                if let Some(PdfAction::Uri(uri_action)) = link.action() {
                    if let Ok(url) = uri_action.uri() {
                        let url_clean = url.trim().to_string();
                        if !url_clean.is_empty() {
                            if let Ok(rect) = link.rect() {
                                let (norm_x, norm_y, norm_w, norm_h) =
                                    normalize_rect(rect, page_w, page_h);
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
                }
            }
        }
    }

    links
}

/// Normalizes PDF points (bottom-left origin) to screen percentage coordinates [0.0, 1.0] (top-left origin).
pub fn normalize_rect(
    rect: pdfium_render::prelude::PdfRect,
    page_w: f32,
    page_h: f32,
) -> (f32, f32, f32, f32) {
    let left = rect.left().value.min(rect.right().value);
    let right = rect.left().value.max(rect.right().value);
    let bottom = rect.bottom().value.min(rect.top().value);
    let top = rect.bottom().value.max(rect.top().value);

    let norm_x = if page_w > 0.0 {
        (left / page_w).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let norm_y = if page_h > 0.0 {
        ((page_h - top) / page_h).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let norm_w = if page_w > 0.0 {
        ((right - left) / page_w).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let norm_h = if page_h > 0.0 {
        ((top - bottom) / page_h).clamp(0.0, 1.0)
    } else {
        0.0
    };

    (norm_x, norm_y, norm_w, norm_h)
}
