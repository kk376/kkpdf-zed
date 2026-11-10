use kkpdf_zed::{PageLayoutMode, PdfToolbarState, PdfView, PdfViewerSettings, PdfiumEngine};
use std::time::Duration;

const MINIMAL_PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000010 00000 n \n0000000060 00000 n \n0000000117 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n185\n%%EOF\n";

#[test]
fn test_end_to_end_pdf_view_lifecycle() {
    let settings = PdfViewerSettings {
        cache_budget_mb: 16,
        ..Default::default()
    };

    let mut view = PdfView::with_engine(settings, PdfiumEngine::with_mock_fallback());
    let sample_pdf = MINIMAL_PDF;

    view.load_from_bytes(sample_pdf.to_vec(), None)
        .expect("Load sample PDF bytes");

    assert_eq!(view.total_pages(), 1);
    assert_eq!(view.display_page_number(), 1);

    // Initial container layout
    view.set_container_size(1024.0, 768.0);
    assert!(view.zoom_percentage() > 0);

    // Zoom manipulations
    view.set_zoom(1.5, Some((512.0, 384.0)));
    assert_eq!(view.zoom_percentage(), 150);

    view.zoom_in(None);
    assert!(view.zoom_percentage() > 150);

    view.reset_zoom();
    assert_eq!(view.zoom_percentage(), 100);

    // Rendering page
    let rendered = view.get_or_render_page(0).expect("Render page 0");
    assert!(rendered.width > 0);
    assert!(rendered.height > 0);
    assert!(rendered.dark_mode); // smart dark mode default

    // Dark mode toggle
    view.toggle_dark_mode();
    let rendered_light = view.get_or_render_page(0).expect("Render light mode");
    assert!(!rendered_light.dark_mode);
}

#[test]
fn test_reload_preserving_exact_viewport_and_zoom() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let pdf_path = temp_dir.path().join("document.pdf");

    // Write initial PDF
    {
        std::fs::write(&pdf_path, MINIMAL_PDF).expect("write");
    }

    let mut view = PdfView::with_engine(
        PdfViewerSettings::default(),
        PdfiumEngine::with_mock_fallback(),
    );
    view.open_file(&pdf_path).expect("open file");

    // Simulate user zooming and panning
    view.set_zoom(2.5, None);
    view.handle_mouse_down((100.0, 100.0));
    view.handle_mouse_move((150.0, 200.0));
    view.handle_mouse_up();

    let initial_zoom = view.zoom_percentage();
    let snapshot = view.save_state_snapshot();
    assert_eq!(initial_zoom, 250);
    assert_eq!(snapshot.pan_x, 50.0);
    assert_eq!(snapshot.pan_y, 100.0);

    // Sleep to ensure timestamp difference
    std::thread::sleep(Duration::from_millis(25));

    // Compiler rebuilds the PDF
    {
        std::fs::write(&pdf_path, MINIMAL_PDF).expect("rebuild");
    }

    // Debounced hot reload trigger
    let reloaded = view.reload_preserving_state().expect("hot reload");
    assert!(reloaded, "Reload should succeed");

    // Verify viewport state was strictly preserved
    assert_eq!(view.zoom_percentage(), initial_zoom);
    let new_snapshot = view.save_state_snapshot();
    assert_eq!(new_snapshot.pan_x, 50.0);
    assert_eq!(new_snapshot.pan_y, 100.0);
}

#[test]
fn test_toolbar_synchronization_with_view_state() {
    let view = PdfView::new(PdfViewerSettings::default());
    let toolbar_state = PdfToolbarState::new(
        view.display_page_number(),
        view.total_pages(),
        view.zoom_percentage(),
        view.dark_mode(),
        view.layout_mode(),
    );

    assert_eq!(toolbar_state.current_page, 1);
    assert_eq!(toolbar_state.zoom_percentage, 100);
    assert!(toolbar_state.dark_mode);
    assert_eq!(toolbar_state.layout_mode, PageLayoutMode::Continuous);
}

#[test]
fn test_fixed_point_tone_mapper_comprehensive_invariants() {
    use kkpdf_zed::LuminosityToneMapper;

    // Buffer layout:
    // Pixel 0: White paper [255, 255, 255, 255] -> should be mapped to dark (~30)
    // Pixel 1: Black ink   [0, 0, 0, 255]       -> should be mapped to light (~215)
    // Pixel 2: Saturated Red [255, 0, 0, 255]   -> must remain untouched
    // Pixel 3: Saturated Green [0, 255, 0, 255] -> must remain untouched
    // Pixel 4: Saturated Blue [0, 0, 255, 255]  -> must remain untouched
    // Pixel 5: Saturated Yellow [255, 255, 0, 255] -> must remain untouched
    // Pixel 6: Fully Transparent [0, 0, 0, 0]   -> must remain untouched
    // Pixel 7: Neutral Gray [128, 128, 128, 255] -> should be inverted
    let mut buffer = vec![
        255, 255, 255, 255,
        0, 0, 0, 255,
        255, 0, 0, 255,
        0, 255, 0, 255,
        0, 0, 255, 255,
        255, 255, 0, 255,
        0, 0, 0, 0,
        128, 128, 128, 255,
    ];

    LuminosityToneMapper::apply(&mut buffer, 0.18);

    // Pixel 0: White paper -> 30
    assert_eq!(buffer[0], 30);
    assert_eq!(buffer[1], 30);
    assert_eq!(buffer[2], 30);
    assert_eq!(buffer[3], 255);

    // Pixel 1: Black ink -> 215
    assert_eq!(buffer[4], 215);
    assert_eq!(buffer[5], 215);
    assert_eq!(buffer[6], 215);
    assert_eq!(buffer[7], 255);

    // Pixel 2: Red preserved
    assert_eq!(buffer[8], 255);
    assert_eq!(buffer[9], 0);
    assert_eq!(buffer[10], 0);

    // Pixel 3: Green preserved
    assert_eq!(buffer[12], 0);
    assert_eq!(buffer[13], 255);
    assert_eq!(buffer[14], 0);

    // Pixel 4: Blue preserved
    assert_eq!(buffer[16], 0);
    assert_eq!(buffer[17], 0);
    assert_eq!(buffer[18], 255);

    // Pixel 5: Yellow preserved
    assert_eq!(buffer[20], 255);
    assert_eq!(buffer[21], 255);
    assert_eq!(buffer[22], 0);

    // Pixel 6: Transparent preserved
    assert_eq!(buffer[24], 0);
    assert_eq!(buffer[25], 0);
    assert_eq!(buffer[26], 0);
    assert_eq!(buffer[27], 0);

    // Pixel 7: Neutral Gray inverted (215 - (128 * 185) / 255 ≈ 122)
    assert!(buffer[28] >= 120 && buffer[28] <= 125);
    assert_eq!(buffer[28], buffer[29]);
    assert_eq!(buffer[29], buffer[30]);
}

#[test]
fn test_spec_compliant_pdf_header_validation_invariants() {
    use kkpdf_zed::pdfium::validate_pdf_bytes;

    // Standard header
    assert!(validate_pdf_bytes(b"%PDF-1.7\n").is_ok());

    // UTF-8 Byte Order Mark (BOM) prefix
    let bom_pdf = b"\xef\xbb\xbf%PDF-1.7\nstream";
    assert!(validate_pdf_bytes(bom_pdf).is_ok());

    // Leading whitespace and export comments
    let commented_pdf = b"% Title: Document\n% Author: Zed\n\r\n\t  %PDF-2.0\n";
    assert!(validate_pdf_bytes(commented_pdf).is_ok());

    // Header within 1024-byte window
    let mut padded_pdf = vec![b' '; 500];
    padded_pdf.extend_from_slice(b"%PDF-1.4\ncontent");
    assert!(validate_pdf_bytes(&padded_pdf).is_ok());

    // Header past 1024-byte window (must be rejected)
    let mut late_pdf = vec![b' '; 1030];
    late_pdf.extend_from_slice(b"%PDF-1.4\ncontent");
    assert!(validate_pdf_bytes(&late_pdf).is_err());

    // Malformed / truncated
    assert!(validate_pdf_bytes(b"").is_err());
    assert!(validate_pdf_bytes(b"%PDF").is_err());
    assert!(validate_pdf_bytes(b"PNG\r\n\x1a\n").is_err());
}

#[test]
fn test_lru_cache_memory_accounting_and_stress_eviction() {
    use kkpdf_zed::cache::{CacheKey, PageLruCache, RenderedPage};

    // Cache with exactly 100 KB budget (minimum 1MB enforced by constructor)
    let budget_bytes = 1024 * 1024; // 1 MB
    let mut cache = PageLruCache::new(budget_bytes);

    // Each page: 256x256x4 = 262,144 bytes (~256 KB)
    // 4 pages = 1,048,576 bytes (exact 1 MB limit)
    let page_width = 256u32;
    let page_height = 256u32;
    let byte_size = (page_width * page_height * 4) as usize;

    for i in 0..10 {
        let key = CacheKey::new(i, 1.0, false);
        let page = RenderedPage::new(
            i,
            page_width,
            page_height,
            1.0,
            false,
            vec![200u8; byte_size],
        );
        cache.insert(key, page);
        assert!(cache.memory_usage() <= budget_bytes);
    }

    // Because each page is 256 KB, only at most 4 pages can fit in 1 MB
    assert!(cache.len() <= 4);
    assert!(cache.memory_usage() <= budget_bytes);

    // Oldest pages (0, 1, 2, 3, 4, 5) must have been evicted
    assert!(cache.get(&CacheKey::new(0, 1.0, false)).is_none());
    assert!(cache.get(&CacheKey::new(1, 1.0, false)).is_none());

    // Most recent pages (e.g. 9) must be present
    assert!(cache.get(&CacheKey::new(9, 1.0, false)).is_some());
}
