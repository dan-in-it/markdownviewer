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
            .with_icon(std::sync::Arc::new(app_icon()))
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

fn app_icon() -> egui::IconData {
    let size: u32 = 64;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let top = (48u8, 44u8, 82u8);
    let bottom = (18u8, 18u8, 32u8);
    for y in 0..size {
        let t = if size > 1 {
            y as f32 / (size - 1) as f32
        } else {
            0.0
        };
        let r = lerp_u8(top.0, bottom.0, t);
        let g = lerp_u8(top.1, bottom.1, t);
        let b = lerp_u8(top.2, bottom.2, t);
        for x in 0..size {
            set_pixel(&mut rgba, size, x as i32, y as i32, r, g, b, 255);
        }
    }

    let border = 1i32.max((size as i32) / 64);
    let border_color = (255u8, 200u8, 80u8, 255u8);
    fill_rect(&mut rgba, size, 0, 0, size as i32, border, border_color);
    fill_rect(
        &mut rgba,
        size,
        0,
        size as i32 - border,
        size as i32,
        border,
        border_color,
    );
    fill_rect(&mut rgba, size, 0, 0, border, size as i32, border_color);
    fill_rect(
        &mut rgba,
        size,
        size as i32 - border,
        0,
        border,
        size as i32,
        border_color,
    );

    let pattern_m = [
        "10001", "11011", "10101", "10001", "10001", "10001", "10001",
    ];
    let pattern_d = [
        "11110", "10001", "10001", "10001", "10001", "10001", "11110",
    ];

    let scale = 2i32.max((size as i32) / 16);
    let letter_w = 5 * scale;
    let letter_h = 7 * scale;
    let spacing = scale;
    let total_w = letter_w * 2 + spacing;
    let x0 = ((size as i32) - total_w) / 2;
    let y0 = ((size as i32) - letter_h) / 2;

    let shadow = (0u8, 0u8, 0u8, 90u8);
    draw_glyph(
        &mut rgba,
        size,
        &pattern_m,
        x0 + border,
        y0 + border,
        scale,
        shadow,
    );
    draw_glyph(
        &mut rgba,
        size,
        &pattern_d,
        x0 + letter_w + spacing + border,
        y0 + border,
        scale,
        shadow,
    );

    let fg = (245u8, 245u8, 245u8, 255u8);
    draw_glyph(&mut rgba, size, &pattern_m, x0, y0, scale, fg);
    draw_glyph(
        &mut rgba,
        size,
        &pattern_d,
        x0 + letter_w + spacing,
        y0,
        scale,
        fg,
    );

    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let a = a as f32;
    let b = b as f32;
    (a * (1.0 - t) + b * t).round().clamp(0.0, 255.0) as u8
}

fn set_pixel(rgba: &mut [u8], size: u32, x: i32, y: i32, r: u8, g: u8, b: u8, a: u8) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= size || y >= size {
        return;
    }
    let i = ((y * size + x) * 4) as usize;
    rgba[i] = r;
    rgba[i + 1] = g;
    rgba[i + 2] = b;
    rgba[i + 3] = a;
}

fn fill_rect(rgba: &mut [u8], size: u32, x: i32, y: i32, w: i32, h: i32, color: (u8, u8, u8, u8)) {
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            set_pixel(rgba, size, xx, yy, color.0, color.1, color.2, color.3);
        }
    }
}

fn draw_glyph(
    rgba: &mut [u8],
    size: u32,
    pattern: &[&str; 7],
    x0: i32,
    y0: i32,
    scale: i32,
    color: (u8, u8, u8, u8),
) {
    for (yy, row) in pattern.iter().enumerate() {
        for (xx, ch) in row.as_bytes().iter().copied().enumerate() {
            if ch != b'1' {
                continue;
            }
            fill_rect(
                rgba,
                size,
                x0 + (xx as i32) * scale,
                y0 + (yy as i32) * scale,
                scale,
                scale,
                color,
            );
        }
    }
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
