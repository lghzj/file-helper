mod app;
mod api;
mod config;
mod logging;
mod models;
mod parser;
mod paths;
mod plugins;
mod storage;
mod watcher;

use app::FileHelperApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--parse") {
        let Some(file) = args.get(2) else {
            eprintln!("用法：file-helper --parse <word文件路径>");
            std::process::exit(2);
        };
        match parser::parse_docx(std::path::Path::new(file), "word_default", "kolin") {
            Ok(parsed) => {
                println!("{}", serde_json::to_string_pretty(&parsed.final_json).unwrap_or_else(|_| "{}".to_owned()));
                return Ok(());
            }
            Err(err) => {
                eprintln!("解析失败：{err}");
                std::process::exit(1);
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Helper")
            .with_inner_size([980.0, 620.0])
            .with_min_inner_size([860.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "File Helper",
        options,
        Box::new(|cc| Ok(Box::new(FileHelperApp::new(cc)))),
    )
}
