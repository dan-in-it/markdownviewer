#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([520.0, 360.0]),
        ..Default::default()
    };

    eframe::run_native(
        "markdownviewer",
        native_options,
        Box::new(|cc| Ok(Box::new(MarkdownViewerApp::new(cc)))),
    )
}

struct MarkdownViewerApp {
    cache: CommonMarkCache,
    markdown: String,
    file_path: Option<PathBuf>,
    error: Option<String>,
}

impl MarkdownViewerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.style_mut(|style| {
            style.url_in_tooltip = true;
        });

        let mut app = Self {
            cache: CommonMarkCache::default(),
            markdown: String::from(
                "# markdownviewer\n\nOpen a `.md` file to view it.\n\n- Use **Open…** or drag and drop a file.\n- Use **Reload** to re-read the current file.\n",
            ),
            file_path: None,
            error: None,
        };

        if let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) {
            let _ = app.load_file(path);
        }

        app
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter(
            "Markdown",
            &["md", "markdown", "mdown", "mkd", "mkdn", "mdtxt"],
        );
        if let Some(path) = &self.file_path {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(path) = dialog.pick_file() {
            let _ = self.load_file(path);
        }
    }

    fn load_file(&mut self, path: PathBuf) -> Result<()> {
        let markdown = read_markdown(&path)?;
        self.cache = CommonMarkCache::default();
        self.markdown = markdown;
        self.file_path = Some(path);
        self.error = None;
        Ok(())
    }

    fn reload(&mut self) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        if let Err(e) = self.load_file(path) {
            self.error = Some(e.to_string());
        }
    }
}

impl eframe::App for MarkdownViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            let _ = self.load_file(path);
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open…").clicked() {
                    self.open_dialog();
                }
                let reload_enabled = self.file_path.is_some();
                if ui
                    .add_enabled(reload_enabled, egui::Button::new("Reload"))
                    .clicked()
                {
                    self.reload();
                }

                ui.separator();

                match &self.file_path {
                    Some(path) => {
                        ui.label(path.display().to_string());
                    }
                    None => {
                        ui.weak("No file loaded");
                    }
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, err);
                ui.separator();
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                CommonMarkViewer::new().show(ui, &mut self.cache, &self.markdown);
            });
        });
    }
}

fn read_markdown(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
