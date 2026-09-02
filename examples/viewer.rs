//! # Standalone PDF Viewer Runner & Benchmark
//!
//! Run with:
//! cargo run -j 2 --example viewer -- [path_to_pdf]
//!
//! Example:
//! cargo run -j 2 --example viewer -- /home/kk376/code/dual_domain_learning_roadmap.pdf

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use kkpdf_zed::cache::{CacheKey, PageLruCache};
use kkpdf_zed::settings::{DefaultZoomPolicy, PageLayoutMode, PdfViewerSettings};
use kkpdf_zed::view::PdfView;

fn main() -> anyhow::Result<()> {
    println!("================================================================================");
    println!("  kkpdf-zed: Standalone Interactive PDF Engine & Benchmark Runner");
    println!("================================================================================");

    // 1. Determine input PDF path
    let args: Vec<String> = env::args().collect();
    let pdf_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        let default_sample = PathBuf::from("/home/kk376/code/dual_domain_learning_roadmap.pdf");
        if default_sample.exists() {
            default_sample
        } else {
            // Create a temporary synthetic PDF if no file provided
            let sample_dir = env::temp_dir().join("kkpdf_test");
            fs::create_dir_all(&sample_dir)?;
            let sample_file = sample_dir.join("sample.pdf");
            let synthetic_pdf = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000010 00000 n \n0000000060 00000 n \n0000000117 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n185\n%%EOF\n";
            fs::write(&sample_file, synthetic_pdf)?;
            sample_file
        }
    };

    println!("[*] Loading PDF: {}", pdf_path.display());
    if !pdf_path.exists() {
        eprintln!("Error: File not found at {}", pdf_path.display());
        std::process::exit(1);
    }

    let file_metadata = fs::metadata(&pdf_path)?;
    println!(
        "    - File Size:          {:.2} KB",
        file_metadata.len() as f64 / 1024.0
    );

    // 2. Initialize PdfView & Parse Document
    let settings = PdfViewerSettings {
        default_zoom: DefaultZoomPolicy::FitWidth,
        layout_mode: PageLayoutMode::Continuous,
        smart_dark_mode: false,
        cache_budget_mb: 128,
        ..Default::default()
    };
    let mut view = PdfView::new(settings);

    let start_load = Instant::now();
    view.open_file(&pdf_path)?;
    let load_duration = start_load.elapsed();

    println!("[+] PDF Loaded & Indexed in {:.2?}:", load_duration);
    println!("    - Total Pages:        {}", view.total_pages());
    println!("    - Initial Page Index: {}", view.current_page());

    // 3. Test Rasterization at 100% Zoom (Light Mode)
    println!("\n[*] Testing Multi-Scale Rasterization & Tone Mapping...");
    let start_render = Instant::now();
    let page1_light = view.get_or_render_page(0)?;
    let render_duration = start_render.elapsed();
    println!(
        "[+] Page 1 Rendered (100% Light Mode): {}x{} px in {:.2?} ({:.2} MB)",
        page1_light.width,
        page1_light.height,
        render_duration,
        page1_light.byte_size() as f64 / (1024.0 * 1024.0)
    );

    // 4. Test Dark Mode Tone Mapping
    view.set_dark_mode(true);
    let start_dark = Instant::now();
    let page1_dark = view.get_or_render_page(0)?;
    let dark_duration = start_dark.elapsed();
    println!(
        "[+] Page 1 Rendered (100% Dark Mode Tone-Mapped): {}x{} px in {:.2?}",
        page1_dark.width, page1_dark.height, dark_duration
    );

    // Save outputs to disk for user visual inspection
    let out_dir = env::current_dir()?.join("target").join("viewer_output");
    fs::create_dir_all(&out_dir)?;
    let light_out_path = out_dir.join("page_1_light.png");
    let dark_out_path = out_dir.join("page_1_dark.png");

    // Save light image
    let light_img: image::ImageBuffer<image::Rgba<u8>, _> = image::ImageBuffer::from_raw(
        page1_light.width,
        page1_light.height,
        page1_light.rgba_buffer.as_ref().clone(),
    )
    .ok_or_else(|| anyhow::anyhow!("Failed to construct ImageBuffer for light mode"))?;
    light_img.save(&light_out_path)?;

    // Save dark image
    let dark_img: image::ImageBuffer<image::Rgba<u8>, _> = image::ImageBuffer::from_raw(
        page1_dark.width,
        page1_dark.height,
        page1_dark.rgba_buffer.as_ref().clone(),
    )
    .ok_or_else(|| anyhow::anyhow!("Failed to construct ImageBuffer for dark mode"))?;
    dark_img.save(&dark_out_path)?;

    println!(
        "    -> Saved Light Mode sample: {}",
        light_out_path.display()
    );
    println!(
        "    -> Saved Dark Mode sample:  {}",
        dark_out_path.display()
    );

    // 5. Test LRU Cache Invariants
    println!("\n[*] Testing Page LRU Memory Cache...");
    let mut cache = PageLruCache::new(64 * 1024 * 1024); // 64 MB budget
    let key_light = CacheKey::new(0, 1.0, false);
    let key_dark = CacheKey::new(0, 1.0, true);
    let key_zoom = CacheKey::new(0, 2.0, false);

    cache.insert(key_light, page1_light);
    cache.insert(key_dark, page1_dark);

    println!(
        "[+] Cache State: {} pages stored ({:.2} MB / {:.2} MB budget)",
        cache.len(),
        cache.memory_usage() as f64 / (1024.0 * 1024.0),
        cache.max_memory() as f64 / (1024.0 * 1024.0)
    );

    assert!(cache.get(&key_light).is_some(), "Cache hit for light mode");
    assert!(cache.get(&key_dark).is_some(), "Cache hit for dark mode");
    assert!(
        cache.get(&key_zoom).is_none(),
        "Cache miss for uncached zoom"
    );
    println!("[+] Cache hit/miss invariants verified successfully!");

    // 6. Test Interactive View State Machine (Zoom, Pan, Layout)
    println!("\n[*] Testing View State Machine (Zoom, Pan, Navigation)...");
    println!(
        "    - Initial Zoom:       {:.1}%",
        view.zoom_level() * 100.0
    );
    println!("    - Initial Page Index: {}", view.current_page());

    // Zoom in with focal point at center (400, 300)
    view.set_zoom(1.5, Some((400.0, 300.0)));
    println!(
        "    - After Zoom In (1.5x at [400, 300]): Zoom = {:.1}%, Pan = ({:.1}, {:.1})",
        view.zoom_level() * 100.0,
        view.pan_offset().0,
        view.pan_offset().1
    );

    // Mouse drag simulation
    view.handle_mouse_down((100.0, 100.0));
    view.handle_mouse_move((150.0, 50.0));
    view.handle_mouse_up();
    println!(
        "    - After Drag (dx=50, dy=-50): Pan = ({:.1}, {:.1})",
        view.pan_offset().0,
        view.pan_offset().1
    );

    // Navigate to next page
    if view.total_pages() > 1 {
        view.next_page();
        println!(
            "    - After Next Page: Current Page Index = {}",
            view.current_page()
        );
    }

    // 7. Test Watcher Debounce & State Preservation
    println!("\n[*] Testing Hot-Reload State Preservation...");
    let snapshot = view.save_state_snapshot();
    println!(
        "    - Snapshot Captured: Page {}, Zoom {:.1}%, Pan ({:.1}, {:.1})",
        snapshot.current_page,
        snapshot.zoom_level * 100.0,
        snapshot.pan_x,
        snapshot.pan_y
    );

    // Create a new view instance from reload and restore snapshot
    let mut reloaded_view = PdfView::new(PdfViewerSettings::default());
    reloaded_view.open_file(&pdf_path)?;
    reloaded_view.restore_state_snapshot(snapshot);

    assert_eq!(reloaded_view.current_page(), view.current_page());
    assert!((reloaded_view.zoom_level() - view.zoom_level()).abs() < 1e-5);
    assert!((reloaded_view.pan_offset().0 - view.pan_offset().0).abs() < 1e-5);
    assert!((reloaded_view.pan_offset().1 - view.pan_offset().1).abs() < 1e-5);
    println!("[+] State restored perfectly across hot reload!");

    println!("\n================================================================================");
    println!("  [SUCCESS] All PDF Engine verification stages passed cleanly!");
    println!("  Generated PNG verification images:");
    println!("    - Light: {}", light_out_path.display());
    println!("    - Dark:  {}", dark_out_path.display());
    println!("================================================================================");

    Ok(())
}
