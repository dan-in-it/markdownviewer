#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::{collections::HashMap, thread};

use anyhow::{Context as _, Result};
use eframe::egui;
use eframe::egui::TextStyle;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use regex::{Captures, Regex};

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

#[derive(Debug, Clone)]
enum SvgState {
    Pending,
    Ready(Arc<[u8]>),
    Error(String),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct MathKey {
    tex: String,
    inline: bool,
    color: egui::Color32,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct MermaidKey {
    source: String,
}

struct MarkdownViewerApp {
    commonmark_cache: CommonMarkCache,
    raw_markdown: String,
    markdown: String,
    file_path: Option<PathBuf>,
    github_repo: Option<GithubRepo>,
    settings: ViewerSettings,
    math_cache: Arc<Mutex<HashMap<MathKey, SvgState>>>,
    math_tx: mpsc::Sender<MathKey>,
    mermaid_cache: Arc<Mutex<HashMap<MermaidKey, SvgState>>>,
    mermaid_tx: mpsc::Sender<MermaidKey>,
    error: Option<String>,
}

impl MarkdownViewerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.style_mut(|style| {
            style.url_in_tooltip = true;
        });

        let math_cache: Arc<Mutex<HashMap<MathKey, SvgState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mermaid_cache: Arc<Mutex<HashMap<MermaidKey, SvgState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (math_tx, math_rx) = mpsc::channel::<MathKey>();
        let (mermaid_tx, mermaid_rx) = mpsc::channel::<MermaidKey>();

        spawn_math_worker(cc.egui_ctx.clone(), math_cache.clone(), math_rx);
        spawn_mermaid_worker(cc.egui_ctx.clone(), mermaid_cache.clone(), mermaid_rx);

        let mut app = Self {
            commonmark_cache: CommonMarkCache::default(),
            raw_markdown: String::from(
                "# markdownviewer\n\nOpen a `.md` file to view it.\n\n- Use **Open…** or drag and drop a file.\n- Use **Reload** to re-read the current file.\n",
            ),
            markdown: String::new(),
            file_path: None,
            github_repo: None,
            settings: ViewerSettings::default(),
            math_cache,
            math_tx,
            mermaid_cache,
            mermaid_tx,
            error: None,
        };

        cc.egui_ctx.set_theme(app.settings.theme_preference);

        app.rebuild_markdown();
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
        self.commonmark_cache = CommonMarkCache::default();
        self.raw_markdown = markdown;
        self.file_path = Some(path);
        self.github_repo = self
            .file_path
            .as_deref()
            .and_then(|p| discover_github_repo(p));
        self.rebuild_markdown();
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

    fn rebuild_markdown(&mut self) {
        self.markdown = preprocess_markdown(
            &self.raw_markdown,
            &self.settings,
            self.github_repo.as_ref(),
        );
        self.commonmark_cache = CommonMarkCache::default();
    }

    fn clear_render_caches(&mut self) {
        self.commonmark_cache = CommonMarkCache::default();
        if let Ok(mut map) = self.math_cache.lock() {
            map.clear();
        }
        if let Ok(mut map) = self.mermaid_cache.lock() {
            map.clear();
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

                let theme_before = self.settings.theme_preference;
                ui.selectable_value(
                    &mut self.settings.theme_preference,
                    egui::ThemePreference::System,
                    "💻",
                )
                .on_hover_text("Follow the system theme");
                ui.selectable_value(
                    &mut self.settings.theme_preference,
                    egui::ThemePreference::Dark,
                    "🌙",
                )
                .on_hover_text("Dark theme");
                ui.selectable_value(
                    &mut self.settings.theme_preference,
                    egui::ThemePreference::Light,
                    "☀",
                )
                .on_hover_text("Light theme");
                if theme_before != self.settings.theme_preference {
                    ctx.set_theme(self.settings.theme_preference);
                    self.clear_render_caches();
                }

                ui.separator();

                ui.menu_button("Options", |ui| {
                    let mut changed = false;
                    changed |= ui
                        .checkbox(
                            &mut self.settings.render_math,
                            "Render math ($...$ / $$...$$)",
                        )
                        .changed();
                    changed |= ui
                        .checkbox(&mut self.settings.render_mermaid, "Render Mermaid diagrams")
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.settings.auto_detect_code_lang,
                            "Auto-detect code languages",
                        )
                        .changed();
                    changed |= ui
                        .checkbox(&mut self.settings.autolink_urls, "Autolink plain URLs")
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.settings.github_links,
                            "GitHub issue/PR links (#123)",
                        )
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.settings.replace_emoji,
                            "Emoji shortcodes (:smile:)",
                        )
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.settings.smart_typography,
                            "Smart typography (off by default)",
                        )
                        .changed();
                    if changed {
                        self.rebuild_markdown();
                    }
                });

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
                let math_cache = self.math_cache.clone();
                let math_tx = self.math_tx.clone();
                let mermaid_cache = self.mermaid_cache.clone();
                let mermaid_tx = self.mermaid_tx.clone();

                let render_math_enabled = self.settings.render_math;
                let render_mermaid_enabled = self.settings.render_mermaid;

                let render_math_fn = move |ui: &mut egui::Ui, tex: &str, inline: bool| {
                    render_math(ui, tex, inline, &math_cache, &math_tx);
                };
                let render_html_fn = move |ui: &mut egui::Ui, html: &str| {
                    render_html(
                        ui,
                        html,
                        render_mermaid_enabled,
                        &mermaid_cache,
                        &mermaid_tx,
                    );
                };

                let mut viewer = CommonMarkViewer::new();
                if render_math_enabled {
                    viewer = viewer.render_math_fn(Some(&render_math_fn));
                }
                if render_mermaid_enabled {
                    viewer = viewer.render_html_fn(Some(&render_html_fn));
                }

                viewer.show(ui, &mut self.commonmark_cache, &self.markdown);
            });
        });
    }
}

fn read_markdown(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

#[derive(Debug, Clone)]
struct GithubRepo {
    base_url: String,
}

fn discover_github_repo(markdown_path: &Path) -> Option<GithubRepo> {
    let dir = markdown_path.parent().unwrap_or(markdown_path);
    let git_root = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !git_root.status.success() {
        return None;
    }
    let git_root = String::from_utf8_lossy(&git_root.stdout).trim().to_string();
    if git_root.is_empty() {
        return None;
    }

    let remote = Command::new("git")
        .arg("-C")
        .arg(&git_root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !remote.status.success() {
        return None;
    }
    let remote = String::from_utf8_lossy(&remote.stdout).trim().to_string();
    parse_github_remote(&remote).map(|base_url| GithubRepo { base_url })
}

fn parse_github_remote(remote: &str) -> Option<String> {
    fn strip_git_suffix(s: &str) -> &str {
        s.strip_suffix(".git").unwrap_or(s)
    }

    if let Some(rest) = remote.strip_prefix("https://github.com/") {
        let rest = strip_git_suffix(rest);
        let mut it = rest.split('/');
        let owner = it.next()?.trim();
        let repo = it.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(format!("https://github.com/{owner}/{repo}"));
    }

    if let Some(rest) = remote.strip_prefix("http://github.com/") {
        let rest = strip_git_suffix(rest);
        let mut it = rest.split('/');
        let owner = it.next()?.trim();
        let repo = it.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(format!("https://github.com/{owner}/{repo}"));
    }

    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        let rest = strip_git_suffix(rest);
        let mut it = rest.split('/');
        let owner = it.next()?.trim();
        let repo = it.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(format!("https://github.com/{owner}/{repo}"));
    }

    if let Some(rest) = remote.strip_prefix("ssh://git@github.com/") {
        let rest = strip_git_suffix(rest);
        let mut it = rest.split('/');
        let owner = it.next()?.trim();
        let repo = it.next()?.trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some(format!("https://github.com/{owner}/{repo}"));
    }

    None
}

#[derive(Debug, Clone)]
struct ViewerSettings {
    theme_preference: egui::ThemePreference,
    render_math: bool,
    render_mermaid: bool,
    auto_detect_code_lang: bool,
    autolink_urls: bool,
    github_links: bool,
    replace_emoji: bool,
    smart_typography: bool,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            theme_preference: egui::ThemePreference::System,
            render_math: true,
            render_mermaid: true,
            auto_detect_code_lang: true,
            autolink_urls: true,
            github_links: true,
            replace_emoji: true,
            smart_typography: false,
        }
    }
}

fn preprocess_markdown(
    input: &str,
    settings: &ViewerSettings,
    github_repo: Option<&GithubRepo>,
) -> String {
    let mut out = String::with_capacity(input.len() + 256);
    let mut fence: Option<FenceState> = None;

    for chunk in input.split_inclusive('\n') {
        if let Some(state) = &mut fence {
            if is_fence_closing_line(chunk, state) {
                flush_fence(&mut out, state, chunk, settings);
                fence = None;
            } else {
                state.content.push_str(chunk);
            }
            continue;
        }

        if let Some(state) = parse_fence_opening_line(chunk) {
            fence = Some(state);
            continue;
        }

        out.push_str(&process_inline_line(chunk, settings, github_repo));
    }

    if let Some(state) = fence {
        out.push_str(&format!(
            "{indent}{fence}{info}\n{content}",
            indent = state.indent,
            fence = state.fence(),
            info = state.info,
            content = state.content
        ));
    }

    out
}

#[derive(Debug, Clone)]
struct FenceState {
    indent: String,
    marker: char,
    marker_len: usize,
    info: String,
    content: String,
}

impl FenceState {
    fn fence(&self) -> String {
        std::iter::repeat(self.marker)
            .take(self.marker_len)
            .collect()
    }
}

fn parse_fence_opening_line(chunk: &str) -> Option<FenceState> {
    let line = chunk.strip_suffix('\n').unwrap_or(chunk);
    let (indent, rest) = line.split_at(line.len() - line.trim_start_matches([' ', '\t']).len());
    let mut chars = rest.chars();
    let marker = chars.next()?;
    if marker != '`' && marker != '~' {
        return None;
    }

    let marker_len = rest.chars().take_while(|&c| c == marker).count();
    if marker_len < 3 {
        return None;
    }

    let after_markers = &rest[marker_len..];
    let info = after_markers.trim().to_string();

    Some(FenceState {
        indent: indent.to_string(),
        marker,
        marker_len,
        info,
        content: String::new(),
    })
}

fn is_fence_closing_line(chunk: &str, state: &FenceState) -> bool {
    let line = chunk.strip_suffix('\n').unwrap_or(chunk);
    let rest = line.trim_start_matches([' ', '\t']);
    if !rest.starts_with(state.marker) {
        return false;
    }
    let run_len = rest.chars().take_while(|&c| c == state.marker).count();
    run_len >= state.marker_len
}

fn flush_fence(
    out: &mut String,
    state: &FenceState,
    closing_line: &str,
    settings: &ViewerSettings,
) {
    let raw_lang = state.info.split_whitespace().next().unwrap_or("");
    if settings.render_mermaid && raw_lang.eq_ignore_ascii_case("mermaid") {
        out.push_str(&state.indent);
        out.push_str("<div class=\"mermaid\">\n");
        out.push_str(&state.content);
        if !state.content.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&state.indent);
        out.push_str("</div>\n");
        return;
    }

    let mut info = raw_lang.to_string();
    info = normalize_fence_language(&info).unwrap_or(info);

    if info.is_empty() && settings.auto_detect_code_lang {
        if let Some(guess) = guess_code_language(&state.content) {
            info = guess.to_string();
        }
    }

    out.push_str(&state.indent);
    out.push_str(&state.fence());
    out.push_str(&info);
    out.push('\n');
    out.push_str(&state.content);
    out.push_str(closing_line);
}

fn normalize_fence_language(lang: &str) -> Option<String> {
    let lang = lang.trim();
    if lang.is_empty() {
        return None;
    }
    let lang_lc = lang.to_ascii_lowercase();
    let mapped = match lang_lc.as_str() {
        "rust" => "rs",
        "python" => "py",
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "bash" | "sh" | "shell" => "sh",
        "powershell" | "pwsh" => "ps1",
        "csharp" | "cs" => "cs",
        "cpp" | "c++" => "cpp",
        "c" => "c",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yml",
        "html" => "html",
        "xml" => "xml",
        "sql" => "sql",
        other => other,
    };
    Some(mapped.to_string())
}

fn guess_code_language(code: &str) -> Option<&'static str> {
    let mut first_non_empty = None;
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        first_non_empty = Some(trimmed);
        break;
    }
    let first = first_non_empty?;

    if let Some(shebang) = first.strip_prefix("#!") {
        let shebang = shebang.to_ascii_lowercase();
        if shebang.contains("python") {
            return Some("py");
        }
        if shebang.contains("bash") || shebang.contains("sh") {
            return Some("sh");
        }
        if shebang.contains("node") {
            return Some("js");
        }
        return Some("sh");
    }

    if first.starts_with("<?xml") {
        return Some("xml");
    }
    if first.starts_with("<!DOCTYPE") || first.starts_with("<html") {
        return Some("html");
    }
    if first.starts_with('{') || first.starts_with('[') {
        return Some("json");
    }
    if first.starts_with("SELECT") || first.starts_with("select") {
        return Some("sql");
    }

    let hay = code;
    if hay.contains("fn main") || hay.contains("println!") || hay.contains("use ") {
        return Some("rs");
    }
    if hay.contains("def ") || hay.contains("import ") {
        return Some("py");
    }
    if hay.contains("console.log") || hay.contains("function ") || hay.contains("=>") {
        return Some("js");
    }
    if hay.contains("using System") || hay.contains("namespace ") {
        return Some("cs");
    }
    if hay.contains("#include") {
        if hay.contains("<iostream>") {
            return Some("cpp");
        }
        return Some("c");
    }

    None
}

fn process_inline_line(
    chunk: &str,
    settings: &ViewerSettings,
    github_repo: Option<&GithubRepo>,
) -> String {
    let (line, line_ending) = chunk
        .strip_suffix('\n')
        .map(|l| (l, "\n"))
        .unwrap_or((chunk, ""));

    let processed = process_inline_text(line, settings, github_repo);
    let mut out = String::with_capacity(processed.len() + line_ending.len());
    out.push_str(&processed);
    out.push_str(line_ending);
    out
}

fn process_inline_text(
    text: &str,
    settings: &ViewerSettings,
    github_repo: Option<&GithubRepo>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_code = false;
    let mut code_delim_len = 0usize;

    while i < text.len() {
        let Some(rel) = text[i..].find('`') else {
            let tail = &text[i..];
            if in_code {
                out.push_str(tail);
            } else {
                out.push_str(&apply_text_transforms(tail, settings, github_repo));
            }
            break;
        };
        let start = i + rel;
        let before = &text[i..start];
        if in_code {
            out.push_str(before);
        } else {
            out.push_str(&apply_text_transforms(before, settings, github_repo));
        }

        let run_len = text[start..].bytes().take_while(|b| *b == b'`').count();
        let run = &text[start..start + run_len];
        out.push_str(run);

        if !in_code {
            in_code = true;
            code_delim_len = run_len;
        } else if run_len == code_delim_len {
            in_code = false;
            code_delim_len = 0;
        }

        i = start + run_len;
    }

    out
}

fn apply_text_transforms(
    text: &str,
    settings: &ViewerSettings,
    github_repo: Option<&GithubRepo>,
) -> String {
    let mut out = text.to_string();

    if settings.github_links {
        out = linkify_github_references(&out, github_repo);
    }
    if settings.autolink_urls {
        out = autolink_plain_urls(&out);
    }
    if settings.replace_emoji {
        out = replace_emoji_shortcodes(&out);
    }
    if settings.smart_typography {
        out = smart_typography(&out);
    }

    out
}

fn linkify_github_references(text: &str, github_repo: Option<&GithubRepo>) -> String {
    static RE_CROSS_REPO: OnceLock<Regex> = OnceLock::new();
    static RE_PR: OnceLock<Regex> = OnceLock::new();
    static RE_ISSUE: OnceLock<Regex> = OnceLock::new();

    let re_cross_repo = RE_CROSS_REPO.get_or_init(|| {
        Regex::new(r"(?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)#(?P<num>[0-9]+)")
            .expect("valid regex")
    });
    let mut out = re_cross_repo
        .replace_all(text, |caps: &Captures| {
            let owner = &caps["owner"];
            let repo = &caps["repo"];
            let num = &caps["num"];
            format!("[{owner}/{repo}#{num}](https://github.com/{owner}/{repo}/issues/{num})")
        })
        .into_owned();

    let Some(repo) = github_repo else {
        return out;
    };

    let re_pr = RE_PR.get_or_init(|| {
        Regex::new(r"(?i)(?P<prefix>^|[^A-Za-z0-9_])PR\s*#(?P<num>[0-9]+)").expect("valid regex")
    });
    out = re_pr
        .replace_all(&out, |caps: &Captures| {
            let prefix = caps.name("prefix").map_or("", |m| m.as_str());
            let num = &caps["num"];
            format!("{prefix}[PR#{num}]({}/pull/{num})", repo.base_url)
        })
        .into_owned();

    let re_issue = RE_ISSUE.get_or_init(|| {
        Regex::new(r"(?P<prefix>^|[^A-Za-z0-9_])#(?P<num>[0-9]+)").expect("valid regex")
    });
    out = re_issue
        .replace_all(&out, |caps: &Captures| {
            let prefix = caps.name("prefix").map_or("", |m| m.as_str());
            let num = &caps["num"];
            format!("{prefix}[#{num}]({}/issues/{num})", repo.base_url)
        })
        .into_owned();

    out
}

fn autolink_plain_urls(text: &str) -> String {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);

    let links: Vec<linkify::Link<'_>> = finder.links(text).collect();
    if links.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + links.len() * 2);
    let mut last = 0usize;

    for link in links {
        let start = link.start();
        let end = link.end();
        out.push_str(&text[last..start]);

        let before = start
            .checked_sub(1)
            .and_then(|i| text.as_bytes().get(i))
            .copied();
        let after = text.as_bytes().get(end).copied();
        if before == Some(b'<') && after == Some(b'>') {
            out.push_str(&text[start..end]);
        } else {
            out.push('<');
            out.push_str(&normalize_autolink_url(link.as_str()));
            out.push('>');
        }

        last = end;
    }

    out.push_str(&text[last..]);
    out
}

fn normalize_autolink_url(url: &str) -> String {
    if url.contains("://") {
        return url.to_string();
    }
    if url.starts_with("mailto:") {
        return url.to_string();
    }
    format!("https://{url}")
}

fn replace_emoji_shortcodes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while let Some(colon1_rel) = text[i..].find(':') {
        let colon1 = i + colon1_rel;
        out.push_str(&text[i..colon1]);

        let rest = &text[colon1 + 1..];
        let Some(colon2_rel) = rest.find(':') else {
            out.push_str(&text[colon1..]);
            return out;
        };
        let name = &rest[..colon2_rel];
        if is_valid_emoji_shortcode(name) {
            if let Some(emoji) = emojis::get_by_shortcode(name) {
                out.push_str(emoji.as_str());
                i = colon1 + 1 + colon2_rel + 1;
                continue;
            }
        }
        out.push(':');
        out.push_str(name);
        out.push(':');

        i = colon1 + 1 + colon2_rel + 1;
    }

    out.push_str(&text[i..]);
    out
}

fn is_valid_emoji_shortcode(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
}

fn smart_typography(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    let mut chars = text.chars().peekable();
    let mut prev_char: Option<char> = None;
    let mut double_open = true;
    let mut single_open = true;

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                let mut dots = 1;
                while dots < 3 && chars.peek().copied() == Some('.') {
                    dots += 1;
                    chars.next();
                }
                if dots == 3 {
                    out.push('…');
                    prev_char = Some('…');
                    continue;
                }
                for _ in 0..dots {
                    out.push('.');
                }
                prev_char = Some('.');
            }
            '-' => {
                if chars.peek().copied() == Some('-') {
                    chars.next();
                    out.push('—');
                    prev_char = Some('—');
                    continue;
                }
                out.push('-');
                prev_char = Some('-');
            }
            '"' => {
                out.push(if double_open { '“' } else { '”' });
                double_open = !double_open;
                prev_char = Some('"');
            }
            '\'' => {
                let next = chars.peek().copied();
                let prev_is_word = prev_char.is_some_and(|c| c.is_alphanumeric());
                let next_is_word = next.is_some_and(|c| c.is_alphanumeric());
                if prev_is_word && next_is_word {
                    out.push('’');
                } else {
                    out.push(if single_open { '‘' } else { '’' });
                    single_open = !single_open;
                }
                prev_char = Some('\'');
            }
            _ => {
                out.push(ch);
                prev_char = Some(ch);
            }
        }
    }

    out
}

fn spawn_math_worker(
    ctx: egui::Context,
    cache: Arc<Mutex<HashMap<MathKey, SvgState>>>,
    rx: mpsc::Receiver<MathKey>,
) {
    thread::spawn(move || {
        while let Ok(key) = rx.recv() {
            let next_state = match render_math_svg(&key) {
                Ok(svg_bytes) => SvgState::Ready(svg_bytes),
                Err(err) => SvgState::Error(err),
            };

            if let Ok(mut map) = cache.lock() {
                map.insert(key, next_state);
            }
            ctx.request_repaint();
        }
    });
}

fn render_math_svg(key: &MathKey) -> std::result::Result<Arc<[u8]>, String> {
    let svg = if key.inline {
        mathjax_svg::convert_to_svg_inline(&key.tex)
    } else {
        mathjax_svg::convert_to_svg(&key.tex)
    }
    .map_err(|e| e.to_string())?;

    let svg = apply_svg_text_color(&svg, key.color);
    Ok(Arc::<[u8]>::from(svg.into_bytes()))
}

fn apply_svg_text_color(svg: &str, color: egui::Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let css = if a == 255 {
        format!("color: rgb({r}, {g}, {b});")
    } else {
        format!("color: rgba({r}, {g}, {b}, {:.3});", (a as f32) / 255.0)
    };

    let Some(svg_start) = svg.find("<svg") else {
        return svg.to_owned();
    };
    let Some(tag_end_rel) = svg[svg_start..].find('>') else {
        return svg.to_owned();
    };
    let tag_end = svg_start + tag_end_rel;
    let start_tag = &svg[svg_start..tag_end];

    if let Some(style_pos_rel) = start_tag.find("style=\"") {
        let value_start = svg_start + style_pos_rel + "style=\"".len();
        let Some(value_end_rel) = svg[value_start..].find('"') else {
            return svg.to_owned();
        };
        let value_end = value_start + value_end_rel;
        let existing = &svg[value_start..value_end];

        let mut out = String::with_capacity(svg.len() + css.len() + 2);
        out.push_str(&svg[..value_end]);
        if !existing.trim().is_empty() && !existing.trim_end().ends_with(';') {
            out.push(';');
        }
        out.push_str(&css);
        out.push_str(&svg[value_end..]);
        return out;
    }

    let mut out = String::with_capacity(svg.len() + css.len() + 16);
    out.push_str(&svg[..tag_end]);
    out.push_str(" style=\"");
    out.push_str(&css);
    out.push('"');
    out.push_str(&svg[tag_end..]);
    out
}

fn spawn_mermaid_worker(
    ctx: egui::Context,
    cache: Arc<Mutex<HashMap<MermaidKey, SvgState>>>,
    rx: mpsc::Receiver<MermaidKey>,
) {
    thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .user_agent("markdownviewer")
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());

        while let Ok(key) = rx.recv() {
            let next_state = match render_mermaid_svg(&client, &key.source) {
                Ok(svg_bytes) => SvgState::Ready(svg_bytes),
                Err(err) => SvgState::Error(err),
            };

            if let Ok(mut map) = cache.lock() {
                map.insert(key, next_state);
            }
            ctx.request_repaint();
        }
    });
}

fn render_mermaid_svg(
    client: &reqwest::blocking::Client,
    source: &str,
) -> std::result::Result<Arc<[u8]>, String> {
    const KROKI_URL: &str = "https://kroki.io/mermaid/svg";

    let response = client
        .post(KROKI_URL)
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(source.to_owned())
        .send()
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body = response.bytes().map_err(|e| e.to_string())?;
    if !status.is_success() {
        let msg = String::from_utf8_lossy(&body).trim().to_string();
        return Err(format!("Kroki returned {status}: {msg}"));
    }

    Ok(Arc::<[u8]>::from(body.to_vec()))
}

fn render_math(
    ui: &mut egui::Ui,
    tex: &str,
    inline: bool,
    cache: &Arc<Mutex<HashMap<MathKey, SvgState>>>,
    tx: &mpsc::Sender<MathKey>,
) {
    let key = MathKey {
        tex: tex.to_owned(),
        inline,
        color: ui.visuals().text_color(),
    };
    let uri = format!("math-{}.svg", egui::Id::new(&key).value());

    let mut should_request = false;
    let state = {
        let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                should_request = true;
                map.insert(key.clone(), SvgState::Pending);
                SvgState::Pending
            }
        }
    };

    if should_request {
        let _ = tx.send(key);
    }

    match state {
        SvgState::Ready(bytes) => {
            let image = egui::Image::new(egui::ImageSource::Bytes {
                uri: uri.into(),
                bytes: egui::load::Bytes::Shared(bytes),
            })
            .fit_to_original_size(1.0);

            if inline {
                ui.add(image.max_height(ui.text_style_height(&TextStyle::Body) * 1.4));
            } else {
                ui.add(image.max_width(ui.available_width()));
            }
        }
        SvgState::Pending => {
            if inline {
                ui.weak("…");
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.weak("Rendering math…");
                });
            }
        }
        SvgState::Error(err) => {
            if inline {
                ui.colored_label(ui.visuals().error_fg_color, "⟂");
            } else {
                ui.colored_label(ui.visuals().error_fg_color, format!("Math error: {err}"));
            }
        }
    }
}

fn render_html(
    ui: &mut egui::Ui,
    html: &str,
    render_mermaid_enabled: bool,
    cache: &Arc<Mutex<HashMap<MermaidKey, SvgState>>>,
    tx: &mpsc::Sender<MermaidKey>,
) {
    if render_mermaid_enabled {
        if let Some(source) = extract_mermaid_source(html) {
            render_mermaid(ui, &source, cache, tx);
            return;
        }
    }

    let mut html_text = html;
    ui.add(
        egui::TextEdit::multiline(&mut html_text)
            .code_editor()
            .desired_width(ui.available_width())
            .desired_rows(1),
    );
}

fn render_mermaid(
    ui: &mut egui::Ui,
    source: &str,
    cache: &Arc<Mutex<HashMap<MermaidKey, SvgState>>>,
    tx: &mpsc::Sender<MermaidKey>,
) {
    let key = MermaidKey {
        source: source.to_owned(),
    };
    let uri = format!("mermaid-{}.svg", egui::Id::new(&key).value());

    let mut should_request = false;
    let state = {
        let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                should_request = true;
                map.insert(key.clone(), SvgState::Pending);
                SvgState::Pending
            }
        }
    };

    if should_request {
        let _ = tx.send(key);
    }

    match state {
        SvgState::Ready(bytes) => {
            ui.add(
                egui::Image::new(egui::ImageSource::Bytes {
                    uri: uri.into(),
                    bytes: egui::load::Bytes::Shared(bytes),
                })
                .fit_to_original_size(1.0)
                .max_width(ui.available_width()),
            );
        }
        SvgState::Pending => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.weak("Rendering Mermaid…");
            });
        }
        SvgState::Error(err) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("Mermaid error: {err}"));
            ui.add_space(4.0);
            ui.collapsing("Show Mermaid source", |ui| {
                let mut src = source;
                ui.add(
                    egui::TextEdit::multiline(&mut src)
                        .code_editor()
                        .desired_width(ui.available_width())
                        .desired_rows(4),
                );
            });
        }
    }
}

fn extract_mermaid_source(html: &str) -> Option<String> {
    let html = html.trim();

    let start = if html.starts_with("<div class=\"mermaid\">") {
        "<div class=\"mermaid\">"
    } else if html.starts_with("<div class='mermaid'>") {
        "<div class='mermaid'>"
    } else {
        return None;
    };

    let inner = html.strip_prefix(start)?;
    let end = inner.rfind("</div>")?;
    let inner = inner[..end].trim_matches(['\n', '\r', ' ', '\t']);

    Some(dedent_block(inner))
}

fn dedent_block(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut min_indent = None::<usize>;

    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        min_indent = Some(min_indent.map_or(indent, |m| m.min(indent)));
    }

    let Some(min_indent) = min_indent else {
        return text.to_string();
    };
    if min_indent == 0 {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    for (idx, line) in lines.iter().enumerate() {
        let mut bytes_idx = 0usize;
        let mut skipped = 0usize;
        for (i, ch) in line.char_indices() {
            if skipped >= min_indent {
                break;
            }
            if ch == ' ' || ch == '\t' {
                bytes_idx = i + ch.len_utf8();
                skipped += 1;
            } else {
                break;
            }
        }
        out.push_str(&line[bytes_idx..]);
        if idx + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}
