//! PDF rasterization tone-mapping and rendering configuration.
//!
//! Provides ITU-R BT.709 luminosity-threshold dark mode color mapping
//! and synthetic fallback page rendering for testing and headless environments.

use crate::cache::RenderedPage;
use crate::document::PageDimensions;
use anyhow::Result;

/// Rendering and tone mapping configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterizerOptions {
    /// Screen or viewport DPI scale (default 72.0 for 1x, 144.0 for 2x Retina).
    pub target_dpi: f32,
    /// User zoom magnification multiplier (1.0 = 100%).
    pub zoom_factor: f32,
    /// Whether dark mode tone mapping is active.
    pub dark_mode: bool,
    /// Saturation threshold (0.0 to 1.0) above which colors are preserved (default 0.18).
    pub saturation_threshold: f32,
}

impl Default for RasterizerOptions {
    fn default() -> Self {
        Self {
            target_dpi: 144.0, // High-DPI default for crisp font rendering
            zoom_factor: 1.0,
            dark_mode: false,
            saturation_threshold: 0.18,
        }
    }
}

/// Luminosity-preserving color remapper for dark themes.
///
/// Unlike naive RGB inversion (`255 - C`) which turns colored photos, figures,
/// and syntax-highlighted diagrams into unusable photo-negatives, this remapper:
/// 1. Computes ITU-R BT.709 relative luminance $Y = 0.2126 R + 0.7152 G + 0.0722 B$.
/// 2. Calculates color saturation $S = \frac{\max(R,G,B) - \min(R,G,B)}{\max(R,G,B)}$.
/// 3. If $S < \text{threshold}$ (neutral paper background or black ink text):
///    - Inverts luminance to match dark theme surface background (`#1E1E2E`) and light text (`#CDD6F4`).
/// 4. If $S \ge \text{threshold}$ (colored graphics, charts, photos):
///    - Leaves pixel RGB channels untouched!
///
/// Precomputed 256-entry lookup table for linear dark theme luminance remapping:
/// 255 (white paper) -> 30 (dark surface), 0 (black ink) -> 215 (light text).
const LUMINANCE_LUT: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        table[i] = (215 - (i * 185).div_ceil(255)) as u8;
        i += 1;
    }
    table
};

pub struct LuminosityToneMapper;

impl LuminosityToneMapper {
    /// Applies dark mode tone mapping in-place on an RGBA8 buffer.
    pub fn apply(rgba_buffer: &mut [u8], saturation_threshold: f32) {
        let thresh_fixed = (saturation_threshold.clamp(0.0, 1.0) * 65536.0 + 0.5) as u32;

        for pixel in rgba_buffer.chunks_exact_mut(4) {
            let a = pixel[3];
            if a == 0 {
                continue;
            }

            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];

            let max_c = r.max(g).max(b);
            let min_c = r.min(g).min(b);

            let diff = (max_c - min_c) as u32;
            let is_desaturated = if diff == 0 {
                true
            } else {
                (diff << 16) < thresh_fixed * (max_c as u32)
            };

            if is_desaturated {
                // Fixed-point ITU-R BT.709 relative luminance (13933 + 46871 + 4732 = 65536)
                let lum = ((13933 * r as u32 + 46871 * g as u32 + 4732 * b as u32 + 32768) >> 16)
                    .min(255) as usize;
                let target_lum = LUMINANCE_LUT[lum];

                pixel[0] = target_lum;
                pixel[1] = target_lum;
                pixel[2] = target_lum;
            }
        }
    }
}

/// Core rasterization pipeline for generating page bitmaps.
pub struct PageRasterizer;

impl PageRasterizer {
    /// Rasterizes a synthetic or mock page buffer for testing and fallback.
    pub fn render_mock_page(
        page_index: usize,
        dimensions: PageDimensions,
        options: RasterizerOptions,
    ) -> Result<RenderedPage> {
        let (width, height) = dimensions.to_pixel_size(options.zoom_factor, options.target_dpi);
        let total_bytes = (width as usize) * (height as usize) * 4;

        let mut buffer = vec![255u8; total_bytes];

        // Draw header bar
        for y in 0..height.min(30) {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                buffer[idx] = 20;
                buffer[idx + 1] = 20;
                buffer[idx + 2] = 20;
                buffer[idx + 3] = 255;
            }
        }

        // Draw saturated red accent box to verify saturation preservation
        for y in 40..height.min(80) {
            for x in 40..width.min(80) {
                let idx = ((y * width + x) * 4) as usize;
                buffer[idx] = 235;
                buffer[idx + 1] = 30;
                buffer[idx + 2] = 30;
                buffer[idx + 3] = 255;
            }
        }

        if options.dark_mode {
            LuminosityToneMapper::apply(&mut buffer, options.saturation_threshold);
        }

        Ok(RenderedPage::new(
            page_index,
            width,
            height,
            options.zoom_factor,
            options.dark_mode,
            buffer,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_luminosity_remapping_white_and_black() {
        // Pixel 0: Pure White [255, 255, 255, 255] (Paper background)
        // Pixel 1: Pure Black [0, 0, 0, 255] (Text)
        let mut buffer = vec![255, 255, 255, 255, 0, 0, 0, 255];

        LuminosityToneMapper::apply(&mut buffer, 0.18);

        // White should be mapped to dark background (~30)
        assert!(
            buffer[0] <= 40,
            "White background should become dark (got {})",
            buffer[0]
        );
        assert_eq!(buffer[0], buffer[1]);
        assert_eq!(buffer[1], buffer[2]);

        // Black should be mapped to light text (~215)
        assert!(
            buffer[4] >= 200,
            "Black text should become light (got {})",
            buffer[4]
        );
        assert_eq!(buffer[4], buffer[5]);
        assert_eq!(buffer[5], buffer[6]);
    }

    #[test]
    fn test_luminosity_remapping_preserves_saturated_colors() {
        // Pixel 0: Saturated Red [255, 0, 0, 255]
        // Pixel 1: Saturated Blue [0, 120, 255, 255]
        let mut buffer = vec![255, 0, 0, 255, 0, 120, 255, 255];

        LuminosityToneMapper::apply(&mut buffer, 0.18);

        // Saturated red must be preserved
        assert_eq!(buffer[0], 255);
        assert_eq!(buffer[1], 0);
        assert_eq!(buffer[2], 0);

        // Saturated blue must be preserved
        assert_eq!(buffer[4], 0);
        assert_eq!(buffer[5], 120);
        assert_eq!(buffer[6], 255);
    }

    #[test]
    fn test_mock_rasterizer_output_sanity() {
        let dim = PageDimensions::new(600.0, 800.0);
        let opts = RasterizerOptions {
            target_dpi: 72.0,
            zoom_factor: 1.0,
            dark_mode: false,
            saturation_threshold: 0.18,
        };

        let page = PageRasterizer::render_mock_page(0, dim, opts).expect("Mock render failed");
        assert_eq!(page.width, 600);
        assert_eq!(page.height, 800);
        assert_eq!(page.rgba_buffer.len(), 600 * 800 * 4);
    }
}
