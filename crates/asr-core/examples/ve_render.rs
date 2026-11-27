//! Pure-Rust CLI for `asr_core::ve_engine` (replaces `scripts/ve.py`).
//! Usage: cargo run -p asr-core --example ve_render -- <spec.json> <out.svg> [--excalidraw <out.excalidraw>] [--icons <icons.json>]

use std::collections::HashMap;
use std::path::PathBuf;

use asr_core::ve_engine;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Usage: ve_render <spec.json> <out.svg> [--excalidraw <out.excalidraw>] [--icons <icons.json>]");
        std::process::exit(2);
    }
    let spec_path = PathBuf::from(&args[0]);
    let svg_path = PathBuf::from(&args[1]);
    let mut exc_path: Option<PathBuf> = None;
    let mut icons_path: Option<PathBuf> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--excalidraw" if i + 1 < args.len() => {
                exc_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--icons" if i + 1 < args.len() => {
                icons_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            other => {
                eprintln!("Unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let icons = match icons_path {
        Some(p) => ve_engine::load_icons_json(&p).unwrap_or_else(|e| {
            eprintln!("✗ icons: {e}");
            std::process::exit(1);
        }),
        None => HashMap::new(),
    };

    match ve_engine::render_spec_file(&spec_path, &icons) {
        Ok(out) => {
            if let Some(parent) = svg_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&svg_path, &out.svg).expect("write svg");
            if let Some(ep) = exc_path {
                std::fs::write(&ep, &out.excalidraw_json).expect("write excalidraw");
            }
            let mark = if out.errors.is_empty() { "✓" } else { "✗" };
            println!(
                "{mark} {}: {} elements · {:.0}×{:.0}",
                spec_path.file_stem().and_then(|s| s.to_str()).unwrap_or("spec"),
                out.element_count,
                out.width,
                out.height
            );
            for err in &out.errors {
                println!("  ✗ {err}");
            }
            for rem in &out.reminders {
                println!("  • {rem}");
            }
            if !out.errors.is_empty() {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("✗ {}: {e}", spec_path.display());
            std::process::exit(1);
        }
    }
}
