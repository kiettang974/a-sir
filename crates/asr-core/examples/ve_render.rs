//! Pure-Rust CLI for `asr_core::ve_engine` (100% replacement for `scripts/ve.py`).
//! Usage:
//!   ve_render --icons
//!   ve_render <spec.json> <out.svg> [--excalidraw <out.excalidraw>] [--theme light|dark|tokens] [--lib <file.excalidrawlib>] [--icons-json <icons.json>]

use std::path::PathBuf;

use asr_core::ve_engine::{self, ThemeMode};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "Usage:\n  ve_render --icons\n  ve_render <spec.json> <out.svg> [--excalidraw <out.excalidraw>] [--theme light|dark|tokens] [--lib <file.excalidrawlib>]"
        );
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--icons") {
        let names = ve_engine::list_icon_names();
        println!("{} built-in icons:", names.len());
        for name in names {
            println!("  {name}");
        }
        return;
    }
    if args.len() < 2 {
        eprintln!("Missing <spec.json> <out.svg>");
        std::process::exit(2);
    }

    let spec_path = PathBuf::from(&args[0]);
    let svg_path = PathBuf::from(&args[1]);
    let mut exc_path: Option<PathBuf> = None;
    let mut theme = ThemeMode::Light;
    let mut icons = ve_engine::builtin_icons().clone();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--excalidraw" if i + 1 < args.len() => {
                exc_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--theme" if i + 1 < args.len() => {
                theme = match args[i + 1].as_str() {
                    "light" => ThemeMode::Light,
                    "dark" => ThemeMode::Dark,
                    "tokens" | "css" => ThemeMode::CssTokens,
                    other => {
                        eprintln!("Unknown theme '{other}' (use light|dark|tokens)");
                        std::process::exit(2);
                    }
                };
                i += 2;
            }
            "--lib" if i + 1 < args.len() => {
                let extra = ve_engine::load_excalidrawlib(&PathBuf::from(&args[i + 1]))
                    .unwrap_or_else(|e| {
                        eprintln!("✗ lib: {e}");
                        std::process::exit(1);
                    });
                icons.extend(extra);
                i += 2;
            }
            "--icons-json" if i + 1 < args.len() => {
                let extra = ve_engine::load_icons_json(&PathBuf::from(&args[i + 1]))
                    .unwrap_or_else(|e| {
                        eprintln!("✗ icons-json: {e}");
                        std::process::exit(1);
                    });
                icons.extend(extra);
                i += 2;
            }
            other => {
                eprintln!("Unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    match ve_engine::render_spec_file_with_theme(&spec_path, &icons, theme) {
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
                spec_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("spec"),
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
