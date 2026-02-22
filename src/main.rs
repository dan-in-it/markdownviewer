#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use eframe::egui;
use eframe::egui::TextStyle;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use rand::Rng as _;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

const STATE_KEY: &str = "markdownviewer_state_v1";
const MAX_RECENT_FILES: usize = 20;

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

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    egui::Color32::from_rgba_unmultiplied(
        lerp_u8(ar, br, t),
        lerp_u8(ag, bg, t),
        lerp_u8(ab, bb, t),
        lerp_u8(aa, ba, t),
    )
}

fn with_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
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

#[derive(Debug, Clone)]
struct OutlineItem {
    level: usize,
    title: String,
    slug: String,
    line: usize,
}

fn slugify_heading(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut pending_dash = false;

    for ch in title.chars() {
        if ch.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            continue;
        }

        if ch.is_whitespace() || ch == '-' || ch == '_' {
            pending_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        match bytes[idx] {
            b'%' if idx + 2 < bytes.len() => {
                let Some(hi) = hex_val(bytes[idx + 1]) else {
                    out.push(bytes[idx]);
                    idx += 1;
                    continue;
                };
                let Some(lo) = hex_val(bytes[idx + 2]) else {
                    out.push(bytes[idx]);
                    idx += 1;
                    continue;
                };
                out.push((hi << 4) | lo);
                idx += 3;
            }
            b'+' => {
                out.push(b' ');
                idx += 1;
            }
            other => {
                out.push(other);
                idx += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn unique_slug(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for idx in 1usize.. {
        let candidate = format!("{base}-{idx}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("infinite loop should always find a unique slug")
}

fn push_outline_item(
    outline: &mut Vec<OutlineItem>,
    used_slugs: &mut HashSet<String>,
    level: usize,
    title: String,
    line: usize,
) {
    let base = slugify_heading(&title);
    let base = if base.is_empty() {
        "section".to_string()
    } else {
        base
    };
    let slug = unique_slug(&base, used_slugs);
    outline.push(OutlineItem {
        level,
        title,
        slug,
        line,
    });
}

fn build_outline(markdown: &str) -> Vec<OutlineItem> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut outline = Vec::new();
    let mut used_slugs = HashSet::<String>::new();

    for (idx, line) in lines.iter().enumerate() {
        let line = line.trim_end_matches(['\r']);
        let trimmed = line.trim_start_matches([' ', '\t']);

        let hash_count = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hash_count) {
            let after_hashes = &trimmed[hash_count..];
            if after_hashes.starts_with([' ', '\t']) {
                let title = after_hashes.trim().trim_end_matches('#').trim().to_string();
                if !title.is_empty() {
                    push_outline_item(&mut outline, &mut used_slugs, hash_count, title, idx);
                }
            }
            continue;
        }

        let underline = trimmed.trim();
        let is_h1 = !underline.is_empty() && underline.chars().all(|c| c == '=');
        let is_h2 = !underline.is_empty() && underline.chars().all(|c| c == '-');
        if (is_h1 || is_h2) && idx > 0 {
            let prev = lines[idx - 1].trim();
            if !prev.is_empty() {
                push_outline_item(
                    &mut outline,
                    &mut used_slugs,
                    if is_h1 { 1 } else { 2 },
                    prev.to_string(),
                    idx - 1,
                );
            }
        }
    }

    outline
}

#[derive(Debug)]
enum WatchCommand {
    SetWatchedFiles(Vec<PathBuf>),
}

fn spawn_file_watcher(
    ctx: egui::Context,
    rx: mpsc::Receiver<WatchCommand>,
    tx: mpsc::Sender<PathBuf>,
) {
    thread::spawn(move || {
        use notify::Watcher as _;

        let event_tx = tx.clone();
        let repaint_ctx = ctx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else {
                    return;
                };
                for path in event.paths {
                    let _ = event_tx.send(path);
                }
                repaint_ctx.request_repaint();
            }) {
                Ok(watcher) => watcher,
                Err(err) => {
                    eprintln!("file watcher disabled: {err}");
                    return;
                }
            };

        let mut watched_dirs = HashSet::<PathBuf>::new();
        while let Ok(cmd) = rx.recv() {
            match cmd {
                WatchCommand::SetWatchedFiles(files) => {
                    let mut next_dirs = HashSet::<PathBuf>::new();
                    for file in files {
                        if let Some(parent) = file.parent() {
                            next_dirs.insert(parent.to_path_buf());
                        }
                    }

                    for dir in watched_dirs.difference(&next_dirs) {
                        let _ = watcher.unwatch(dir);
                    }
                    for dir in next_dirs.difference(&watched_dirs) {
                        let _ = watcher.watch(dir, notify::RecursiveMode::NonRecursive);
                    }

                    watched_dirs = next_dirs;
                }
            }
        }
    });
}

struct Document {
    id: u64,
    raw_markdown: String,
    markdown: String,
    file_path: Option<PathBuf>,
    github_repo: Option<GithubRepo>,
    outline: Vec<OutlineItem>,
    scroll_to_line: Option<usize>,
    commonmark_cache: CommonMarkCache,
}

impl Document {
    fn welcome(id: u64, settings: &ViewerSettings) -> Self {
        let raw_markdown = String::from(
            "# markdownviewer\n\nOpen a `.md` file to view it.\n\n- Use **Open…** or drag and drop a file.\n- Use **Reload** to re-read the current file.\n",
        );
        Self::from_content(id, raw_markdown, None, settings)
    }

    fn from_path(id: u64, path: PathBuf, settings: &ViewerSettings) -> Result<Self> {
        let raw_markdown = read_markdown(&path)?;
        Ok(Self::from_content(id, raw_markdown, Some(path), settings))
    }

    fn from_content(
        id: u64,
        raw_markdown: String,
        file_path: Option<PathBuf>,
        settings: &ViewerSettings,
    ) -> Self {
        let github_repo = file_path.as_deref().and_then(|p| discover_github_repo(p));

        let mut doc = Self {
            id,
            raw_markdown,
            markdown: String::new(),
            file_path,
            github_repo,
            outline: Vec::new(),
            scroll_to_line: None,
            commonmark_cache: CommonMarkCache::default(),
        };
        doc.rebuild_markdown(settings);
        doc
    }

    fn rebuild_markdown(&mut self, settings: &ViewerSettings) {
        self.markdown =
            preprocess_markdown(&self.raw_markdown, settings, self.github_repo.as_ref());
        self.outline = build_outline(&self.raw_markdown);
        self.commonmark_cache = CommonMarkCache::default();
    }

    fn reload(&mut self, settings: &ViewerSettings) -> Result<()> {
        let path = self.file_path.clone().context("no file to reload")?;
        self.raw_markdown = read_markdown(&path)?;
        self.github_repo = discover_github_repo(&path);
        self.rebuild_markdown(settings);
        Ok(())
    }

    fn display_name(&self) -> String {
        match &self.file_path {
            Some(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => "Welcome".to_string(),
        }
    }

    fn tooltip(&self) -> Option<String> {
        self.file_path.as_ref().map(|p| p.display().to_string())
    }

    fn heading_line_for_fragment(&self, fragment: &str) -> Option<usize> {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            return Some(0);
        }

        let decoded = percent_decode(fragment).trim().to_string();
        if decoded.is_empty() {
            return Some(0);
        }

        let decoded = decoded
            .strip_prefix("user-content-")
            .unwrap_or(decoded.as_str())
            .trim();
        if decoded.is_empty() {
            return Some(0);
        }

        let decoded_lc = decoded.to_lowercase();
        if let Some(item) = self.outline.iter().find(|item| item.slug == decoded_lc) {
            return Some(item.line);
        }

        let slug = slugify_heading(decoded);
        if slug.is_empty() {
            return None;
        }

        self.outline
            .iter()
            .find(|item| item.slug == slug)
            .map(|item| item.line)
    }
}

#[derive(Debug, Clone)]
struct FindMatch {
    line: usize,
    col: usize,
    preview: String,
}

#[derive(Debug, Default)]
struct FindState {
    open: bool,
    focus_query: bool,
    query: String,
    case_sensitive: bool,
    matches: Vec<FindMatch>,
    selected: usize,
    last_doc_id: Option<u64>,
    last_query: String,
    last_case_sensitive: bool,
}

fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<FindMatch> {
    const MAX_MATCHES: usize = 500;
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let query_cmp = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    for (line_no, line) in text.lines().enumerate() {
        let hay = if case_sensitive {
            line.to_string()
        } else {
            line.to_lowercase()
        };

        let mut start = 0usize;
        while start <= hay.len() {
            let Some(rel) = hay[start..].find(&query_cmp) else {
                break;
            };
            let col = start + rel;
            let mut preview = line.trim().to_string();
            if preview.len() > 180 {
                preview.truncate(180);
                preview.push('…');
            }

            matches.push(FindMatch {
                line: line_no,
                col,
                preview,
            });
            if matches.len() >= MAX_MATCHES {
                return matches;
            }

            start = col + query_cmp.len().max(1);
        }
    }

    matches
}

struct MarkdownViewerApp {
    documents: Vec<Document>,
    active_doc: usize,
    settings: ViewerSettings,
    persisted: PersistedState,
    next_doc_id: u64,
    find: FindState,
    watch_cmd_tx: mpsc::Sender<WatchCommand>,
    watch_event_rx: mpsc::Receiver<PathBuf>,
    pending_reloads: HashMap<PathBuf, Instant>,
    math_cache: Arc<Mutex<HashMap<MathKey, SvgState>>>,
    math_tx: mpsc::Sender<MathKey>,
    mermaid_cache: Arc<Mutex<HashMap<MermaidKey, SvgState>>>,
    mermaid_tx: mpsc::Sender<MermaidKey>,
    error: Option<String>,
    editor_has_focus: bool,
    drop_zone_visible: bool,
    image_config: ImageConfig,
    notifications: Vec<Notification>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageStorageMode {
    Local,
    Base64,
    Remote,
}

#[derive(Debug, Clone)]
struct ImageConfig {
    storage_mode: ImageStorageMode,
    local_path: String,
    remote_endpoint: Option<String>,
    max_file_size_mb: usize,
    allowed_types: Vec<&'static str>,
    base64_max_size_kb: usize,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            storage_mode: ImageStorageMode::Local,
            local_path: "./assets/images/".to_string(),
            remote_endpoint: None,
            max_file_size_mb: 10,
            allowed_types: vec![
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "image/svg+xml",
            ],
            base64_max_size_kb: 500,
        }
    }
}

#[derive(Debug, Clone)]
struct PendingImage {
    name: String,
    mime: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Notification {
    message: String,
    created_at: Instant,
}

impl MarkdownViewerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.style_mut(|style| {
            style.url_in_tooltip = true;
        });

        let persisted: PersistedState = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, STATE_KEY))
            .unwrap_or_default();
        let settings = persisted.settings.clone();

        let math_cache: Arc<Mutex<HashMap<MathKey, SvgState>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mermaid_cache: Arc<Mutex<HashMap<MermaidKey, SvgState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (math_tx, math_rx) = mpsc::channel::<MathKey>();
        let (mermaid_tx, mermaid_rx) = mpsc::channel::<MermaidKey>();

        spawn_math_worker(cc.egui_ctx.clone(), math_cache.clone(), math_rx);
        spawn_mermaid_worker(cc.egui_ctx.clone(), mermaid_cache.clone(), mermaid_rx);

        let (watch_cmd_tx, watch_cmd_rx) = mpsc::channel::<WatchCommand>();
        let (watch_event_tx, watch_event_rx) = mpsc::channel::<PathBuf>();
        spawn_file_watcher(cc.egui_ctx.clone(), watch_cmd_rx, watch_event_tx);

        let mut app = Self {
            documents: Vec::new(),
            active_doc: 0,
            settings,
            persisted,
            next_doc_id: 1,
            find: FindState::default(),
            watch_cmd_tx,
            watch_event_rx,
            pending_reloads: HashMap::new(),
            math_cache,
            math_tx,
            mermaid_cache,
            mermaid_tx,
            error: None,
            editor_has_focus: false,
            drop_zone_visible: false,
            image_config: ImageConfig::default(),
            notifications: Vec::new(),
        };

        apply_app_theme(&cc.egui_ctx, app.settings.theme);

        let startup_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if !startup_paths.is_empty() {
            for path in startup_paths {
                let _ = app.open_file(path);
            }
        } else {
            let open_files = app.persisted.open_files.clone();
            for path in open_files {
                let _ = app.open_file(path);
            }
            if let Some(active_path) = app.persisted.active_file.clone() {
                if let Some(idx) = app
                    .documents
                    .iter()
                    .position(|doc| doc.file_path.as_ref() == Some(&active_path))
                {
                    app.active_doc = idx;
                }
            }
        }

        if app.documents.is_empty() {
            let id = app.alloc_doc_id();
            app.documents.push(Document::welcome(id, &app.settings));
            app.active_doc = 0;
        }

        app.update_watched_paths();

        app
    }

    fn alloc_doc_id(&mut self) -> u64 {
        let id = self.next_doc_id;
        self.next_doc_id = self.next_doc_id.saturating_add(1);
        id
    }

    fn refresh_find_cache(&mut self) {
        let Some(doc) = self.active_document() else {
            self.find.matches.clear();
            self.find.selected = 0;
            self.find.last_doc_id = None;
            return;
        };

        let doc_id = doc.id;
        if self.find.last_doc_id != Some(doc_id)
            || self.find.last_query != self.find.query
            || self.find.last_case_sensitive != self.find.case_sensitive
        {
            self.find.matches = find_matches(
                &doc.raw_markdown,
                &self.find.query,
                self.find.case_sensitive,
            );
            self.find.selected = 0;
            self.find.last_doc_id = Some(doc_id);
            self.find.last_query = self.find.query.clone();
            self.find.last_case_sensitive = self.find.case_sensitive;
        }

        if !self.find.matches.is_empty() && self.find.selected >= self.find.matches.len() {
            self.find.selected = self.find.matches.len() - 1;
        }
    }

    fn find_next(&mut self) {
        self.refresh_find_cache();
        if self.find.matches.is_empty() {
            return;
        }
        self.find.selected = (self.find.selected + 1) % self.find.matches.len();
        self.jump_to_find_selected();
    }

    fn find_prev(&mut self) {
        self.refresh_find_cache();
        if self.find.matches.is_empty() {
            return;
        }
        if self.find.selected == 0 {
            self.find.selected = self.find.matches.len() - 1;
        } else {
            self.find.selected -= 1;
        }
        self.jump_to_find_selected();
    }

    fn jump_to_find_selected(&mut self) {
        let Some(line) = self.find.matches.get(self.find.selected).map(|m| m.line) else {
            return;
        };
        if let Some(doc) = self.active_document_mut() {
            doc.scroll_to_line = Some(line);
        }
    }

    fn show_find_window(&mut self, ctx: &egui::Context) {
        if !self.find.open {
            return;
        }

        self.refresh_find_cache();
        let matches = self.find.matches.clone();
        let selected = self.find.selected;

        let mut jump_to_line = None::<usize>;
        let mut do_next = false;
        let mut do_prev = false;

        egui::Window::new("Find")
            .open(&mut self.find.open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.text_edit_singleline(&mut self.find.query);
                    if self.find.focus_query {
                        resp.request_focus();
                        self.find.focus_query = false;
                    }

                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        do_next = true;
                    }

                    if ui.checkbox(&mut self.find.case_sensitive, "Aa").changed() {
                        // Updated below after the window closes.
                    }

                    if ui.button("Prev").clicked() {
                        do_prev = true;
                    }
                    if ui.button("Next").clicked() {
                        do_next = true;
                    }
                });

                if self.find.query.is_empty() {
                    ui.weak("Type to search…");
                } else {
                    ui.label(format!(
                        "{} match{}",
                        matches.len(),
                        if matches.len() == 1 { "" } else { "es" }
                    ));
                }

                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for (idx, m) in matches.iter().enumerate() {
                            let label = format!("{}:{}  {}", m.line + 1, m.col + 1, m.preview);
                            if ui.selectable_label(idx == selected, label).clicked() {
                                self.find.selected = idx;
                                jump_to_line = Some(m.line);
                            }
                        }
                    });
            });

        // Update cached results for changes made in the find window this frame.
        self.refresh_find_cache();

        if let Some(line) = jump_to_line {
            if let Some(doc) = self.active_document_mut() {
                doc.scroll_to_line = Some(line);
            }
        }
        if do_prev {
            self.find_prev();
        } else if do_next {
            self.find_next();
        }
    }

    fn update_watched_paths(&mut self) {
        if !self.settings.auto_reload {
            self.pending_reloads.clear();
            let _ = self
                .watch_cmd_tx
                .send(WatchCommand::SetWatchedFiles(Vec::new()));
            return;
        }

        let files: Vec<PathBuf> = self
            .documents
            .iter()
            .filter_map(|doc| doc.file_path.clone())
            .collect();
        let _ = self.watch_cmd_tx.send(WatchCommand::SetWatchedFiles(files));
    }

    fn pump_watch_events(&mut self) {
        while let Ok(path) = self.watch_event_rx.try_recv() {
            let normalized = normalize_path(path.clone());
            for doc_path in self
                .documents
                .iter()
                .filter_map(|doc| doc.file_path.clone())
            {
                if doc_path == path || doc_path == normalized {
                    self.pending_reloads.insert(doc_path, Instant::now());
                }
            }
        }
    }

    fn process_pending_reloads(&mut self) {
        const DEBOUNCE: Duration = Duration::from_millis(250);
        let now = Instant::now();

        let ready: Vec<PathBuf> = self
            .pending_reloads
            .iter()
            .filter_map(|(path, last_change)| {
                if now.duration_since(*last_change) >= DEBOUNCE {
                    Some(path.clone())
                } else {
                    None
                }
            })
            .collect();

        for path in ready {
            self.pending_reloads.remove(&path);
            self.reload_open_file_silently(&path);
        }
    }

    fn reload_open_file_silently(&mut self, path: &Path) {
        let settings = self.settings.clone();
        let Some(idx) = self
            .documents
            .iter()
            .position(|doc| doc.file_path.as_deref() == Some(path))
        else {
            return;
        };

        if self.documents[idx].reload(&settings).is_ok() && idx == self.active_doc {
            self.error = None;
        }
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().add_filter(
            "Markdown",
            &["md", "markdown", "mdown", "mkd", "mkdn", "mdtxt"],
        );
        if let Some(path) = self.active_document().and_then(|d| d.file_path.as_ref()) {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
        }
        if let Some(paths) = dialog.pick_files() {
            for path in paths {
                let _ = self.open_file(path);
            }
        }
    }

    fn open_file(&mut self, path: PathBuf) -> Result<()> {
        let path = normalize_path(path);
        if let Some(idx) = self
            .documents
            .iter()
            .position(|doc| doc.file_path.as_ref() == Some(&path))
        {
            self.active_doc = idx;
            if let Err(err) = self.documents[idx].reload(&self.settings) {
                self.error = Some(err.to_string());
            } else {
                self.error = None;
                self.persisted.remember_file(path);
                self.update_watched_paths();
            }
            return Ok(());
        }

        match Document::from_path(self.alloc_doc_id(), path, &self.settings) {
            Ok(doc) => {
                self.documents.push(doc);
                self.active_doc = self.documents.len().saturating_sub(1);
                self.error = None;
                if let Some(path) = self.documents[self.active_doc].file_path.clone() {
                    self.persisted.remember_file(path);
                }
                self.update_watched_paths();
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
        Ok(())
    }

    fn reload_active(&mut self) {
        let settings = self.settings.clone();
        let Some(doc) = self.active_document_mut() else {
            return;
        };
        if let Err(e) = doc.reload(&settings) {
            self.error = Some(e.to_string());
        } else {
            self.error = None;
        }
    }

    fn rebuild_all_markdown(&mut self) {
        for doc in &mut self.documents {
            doc.rebuild_markdown(&self.settings);
        }
    }

    fn clear_render_caches(&mut self) {
        for doc in &mut self.documents {
            doc.commonmark_cache = CommonMarkCache::default();
        }
        if let Ok(mut map) = self.math_cache.lock() {
            map.clear();
        }
        if let Ok(mut map) = self.mermaid_cache.lock() {
            map.clear();
        }
    }

    fn active_document(&self) -> Option<&Document> {
        self.documents.get(self.active_doc)
    }

    fn active_document_mut(&mut self) -> Option<&mut Document> {
        self.documents.get_mut(self.active_doc)
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.documents.len() {
            return;
        }
        self.documents.remove(idx);
        if self.documents.is_empty() {
            let id = self.alloc_doc_id();
            self.documents.push(Document::welcome(id, &self.settings));
            self.active_doc = 0;
        } else if self.active_doc >= self.documents.len() {
            self.active_doc = self.documents.len() - 1;
        }
        self.update_watched_paths();
    }

    fn show_tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut close_tab = None::<usize>;
        egui::ScrollArea::horizontal()
            .id_salt("tab_bar_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (idx, doc) in self.documents.iter().enumerate() {
                        let is_active = idx == self.active_doc;
                        let fill = if is_active {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        };
                        let stroke_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
                        let response = egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0, stroke_color))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_min_width(120.0);
                                ui.horizontal(|ui| {
                                    let label = doc.display_name();
                                    let mut label_resp =
                                        ui.selectable_label(is_active, label.clone());
                                    if let Some(tt) = doc.tooltip() {
                                        label_resp = label_resp.on_hover_text(tt);
                                    }
                                    if label_resp.clicked() {
                                        self.active_doc = idx;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new("×")
                                                .small()
                                                .fill(egui::Color32::TRANSPARENT),
                                        )
                                        .on_hover_text("Close tab")
                                        .clicked()
                                    {
                                        close_tab = Some(idx);
                                    }
                                });
                            })
                            .response;
                        if response.clicked() {
                            self.active_doc = idx;
                        }
                    }
                });
            });
        if let Some(idx) = close_tab {
            self.close_tab(idx);
        }
    }

    fn handle_internal_anchor_links(&mut self, ctx: &egui::Context) {
        let mut fragments = Vec::<String>::new();
        ctx.output_mut(|o| {
            o.commands.retain(|cmd| {
                let egui::OutputCommand::OpenUrl(open) = cmd else {
                    return true;
                };
                let Some(fragment) = open.url.strip_prefix('#') else {
                    return true;
                };
                fragments.push(fragment.to_string());
                false
            });
        });

        let Some(fragment) = fragments.pop() else {
            return;
        };
        let Some(doc) = self.active_document_mut() else {
            return;
        };
        let Some(line) = doc.heading_line_for_fragment(&fragment) else {
            return;
        };

        doc.scroll_to_line = Some(line);
        ctx.request_repaint();
    }

    fn push_error(&mut self, message: impl Into<String>) {
        self.notifications.push(Notification {
            message: message.into(),
            created_at: Instant::now(),
        });
    }

    fn process_images_for_active_doc(&mut self, files: Vec<PendingImage>) {
        if files.is_empty() {
            return;
        }
        let settings = self.settings.clone();
        let image_config = self.image_config.clone();
        for file in files {
            if !image_config.allowed_types.iter().any(|t| *t == file.mime) {
                self.push_error("Only image files are supported.");
                continue;
            }
            let max_bytes = image_config.max_file_size_mb * 1024 * 1024;
            if file.bytes.len() > max_bytes {
                self.push_error(format!(
                    "Image exceeds maximum size of {}MB.",
                    image_config.max_file_size_mb
                ));
                continue;
            }

            if self.documents.get(self.active_doc).is_none() {
                continue;
            }
            let placeholder_id = rand::rng().random::<u32>();
            let placeholder = format!("![Uploading image...](placeholder-{placeholder_id})");
            {
                let doc = &mut self.documents[self.active_doc];
                if !doc.raw_markdown.ends_with('\n') {
                    doc.raw_markdown.push('\n');
                }
                doc.raw_markdown.push_str(&placeholder);
                doc.raw_markdown.push('\n');
            }

            let store_result = {
                let doc = &self.documents[self.active_doc];
                store_image_for_doc(doc, &image_config, &file)
            };

            let final_markdown = match store_result {
                Ok(markdown) => markdown,
                Err(err) => {
                    self.push_error(err.to_string());
                    format!("<!-- image upload failed: {} -->", file.name)
                }
            };

            let doc = &mut self.documents[self.active_doc];
            doc.raw_markdown = doc.raw_markdown.replacen(&placeholder, &final_markdown, 1);
            doc.rebuild_markdown(&settings);
        }
    }

    fn show_notifications(&mut self, ctx: &egui::Context) {
        self.notifications
            .retain(|n| n.created_at.elapsed() < Duration::from_secs(5));
        if self.notifications.is_empty() {
            return;
        }
        egui::Area::new("error_toasts".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    for note in &self.notifications {
                        egui::Frame::new()
                            .fill(egui::Color32::from_rgb(90, 20, 20))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.colored_label(
                                    egui::Color32::from_rgb(255, 220, 220),
                                    &note.message,
                                );
                            });
                    }
                });
            });
    }
}

fn store_image_for_doc(doc: &Document, cfg: &ImageConfig, file: &PendingImage) -> Result<String> {
    let ext = Path::new(&file.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    let random = format!("{:04x}", rand::rng().random::<u16>());
    let mut filename = format!("image-{}-{random}.{ext}", chrono_like_timestamp());
    match cfg.storage_mode {
        ImageStorageMode::Local => {
            let doc_dir = doc
                .file_path
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or_else(|| Path::new("."));
            let base_dir = doc_dir.join(cfg.local_path.trim_start_matches("./"));
            fs::create_dir_all(&base_dir)?;
            let mut output = base_dir.join(&filename);
            while output.exists() {
                filename = format!(
                    "image-{}-{:06x}.{ext}",
                    chrono_like_timestamp(),
                    rand::rng().random::<u32>() & 0x00ff_ffff
                );
                output = base_dir.join(&filename);
            }
            let mut f = fs::File::create(output)?;
            f.write_all(&file.bytes)?;
            Ok(format!("![](./assets/images/{filename})"))
        }
        ImageStorageMode::Base64 => {
            if file.bytes.len() > cfg.base64_max_size_kb * 1024 {
                anyhow::bail!("Image too large for base64 mode.")
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(&file.bytes);
            Ok(format!("![](data:{};base64,{encoded})", file.mime))
        }
        ImageStorageMode::Remote => {
            let endpoint = cfg
                .remote_endpoint
                .as_ref()
                .context("remote endpoint is not configured")?;
            let resp = reqwest::blocking::Client::new()
                .post(endpoint)
                .body(file.bytes.clone())
                .send()?;
            let url = resp.text()?;
            Ok(format!("![]({})", url.trim()))
        }
    }
}

fn chrono_like_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl eframe::App for MarkdownViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped_paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        let dropped_images: Vec<PendingImage> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| {
                    let mime = f.mime.clone();
                    let bytes = f.bytes.as_ref()?.to_vec();
                    Some(PendingImage {
                        name: if f.name.is_empty() {
                            f.path
                                .as_ref()
                                .and_then(|p| {
                                    p.file_name().map(|n| n.to_string_lossy().into_owned())
                                })
                                .unwrap_or_else(|| "image.png".to_string())
                        } else {
                            f.name.clone()
                        },
                        mime,
                        bytes,
                    })
                })
                .collect()
        });

        self.drop_zone_visible = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drop_zone_visible = false;
        }
        if self.editor_has_focus {
            self.process_images_for_active_doc(dropped_images);
            for path in dropped_paths {
                if path.is_file() {
                    match fs::read(&path) {
                        Ok(bytes) => {
                            let mime = infer_mime_from_path(&path);
                            self.process_images_for_active_doc(vec![PendingImage {
                                name: path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "image.png".to_string()),
                                mime,
                                bytes,
                            }]);
                        }
                        Err(err) => self.push_error(format!("Failed to read dropped file: {err}")),
                    }
                }
            }
        } else {
            for path in dropped_paths {
                let _ = self.open_file(path);
            }
        }

        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::I)) {
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"])
                .pick_files()
            {
                let mut pending = Vec::new();
                for path in files {
                    if let Ok(bytes) = fs::read(&path) {
                        pending.push(PendingImage {
                            name: path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            mime: infer_mime_from_path(&path),
                            bytes,
                        });
                    }
                }
                self.process_images_for_active_doc(pending);
            }
        }

        if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.command) {
            self.find.open = true;
            self.find.focus_query = true;
        }
        if self.find.open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.find.open = false;
        }

        if self.settings.auto_reload {
            self.pump_watch_events();
            self.process_pending_reloads();
        } else {
            while self.watch_event_rx.try_recv().is_ok() {}
            self.pending_reloads.clear();
        }

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open…").clicked() {
                        self.open_dialog();
                    }
                    if ui.button("🖼 Upload image").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"])
                            .pick_files()
                        {
                            let mut pending = Vec::new();
                            for path in files {
                                if let Ok(bytes) = fs::read(&path) {
                                    pending.push(PendingImage {
                                        name: path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default(),
                                        mime: infer_mime_from_path(&path),
                                        bytes,
                                    });
                                }
                            }
                            self.process_images_for_active_doc(pending);
                        }
                    }
                    ui.menu_button("Recent", |ui| {
                        let recent = self.persisted.recent_files.clone();
                        if recent.is_empty() {
                            ui.weak("No recent files");
                            return;
                        }

                        for path in recent {
                            let exists = path.is_file();
                            let label = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());

                            if ui
                                .add_enabled(exists, egui::Button::new(label))
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                let _ = self.open_file(path);
                                ui.close();
                            }
                        }

                        ui.separator();
                        if ui.button("Clear recent").clicked() {
                            self.persisted.recent_files.clear();
                            ui.close();
                        }
                    });
                    let reload_enabled = self
                        .active_document()
                        .and_then(|d| d.file_path.as_ref())
                        .is_some();
                    if ui
                        .add_enabled(reload_enabled, egui::Button::new("Reload"))
                        .clicked()
                    {
                        self.reload_active();
                    }
                    if ui.button("Find…").on_hover_text("Ctrl+F").clicked() {
                        self.find.open = true;
                        self.find.focus_query = true;
                    }

                    ui.separator();

                    let mut auto_reload_changed = false;
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
                        ui.separator();
                        ui.checkbox(&mut self.settings.show_outline, "Show outline panel");
                        auto_reload_changed |= ui
                            .checkbox(&mut self.settings.auto_reload, "Auto-reload changed files")
                            .changed();
                        ui.separator();
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
                        ui.separator();
                        ui.label("Image storage mode");
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.image_config.storage_mode,
                                ImageStorageMode::Local,
                                "Local",
                            );
                            ui.selectable_value(
                                &mut self.image_config.storage_mode,
                                ImageStorageMode::Base64,
                                "Base64",
                            );
                            ui.selectable_value(
                                &mut self.image_config.storage_mode,
                                ImageStorageMode::Remote,
                                "Remote",
                            );
                        });
                        if changed {
                            self.rebuild_all_markdown();
                        }
                    });
                    if auto_reload_changed {
                        self.update_watched_paths();
                    }

                    ui.separator();

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let theme_before = self.settings.theme;
                        ui.selectable_value(
                            &mut self.settings.theme,
                            AppTheme::TerminalAmber,
                            egui::RichText::new("A>")
                                .monospace()
                                .color(egui::Color32::from_rgb(0xff, 0xb0, 0x36)),
                        )
                        .on_hover_text("Amber terminal theme");
                        ui.selectable_value(
                            &mut self.settings.theme,
                            AppTheme::TerminalGreen,
                            egui::RichText::new("G>")
                                .monospace()
                                .color(egui::Color32::from_rgb(0x00, 0xff, 0x7a)),
                        )
                        .on_hover_text("Green terminal theme");
                        ui.selectable_value(&mut self.settings.theme, AppTheme::Light, "☀")
                            .on_hover_text("Light theme");
                        ui.selectable_value(&mut self.settings.theme, AppTheme::Dark, "🌙")
                            .on_hover_text("Dark theme");
                        ui.selectable_value(&mut self.settings.theme, AppTheme::System, "💻")
                            .on_hover_text("Follow the system theme");
                        if theme_before != self.settings.theme {
                            apply_app_theme(ctx, self.settings.theme);
                            self.clear_render_caches();
                        }

                        ui.separator();

                        match self
                            .active_document()
                            .and_then(|doc| doc.file_path.as_ref())
                        {
                            Some(path) => {
                                ui.add(egui::Label::new(path.display().to_string()).truncate());
                            }
                            None => {
                                ui.weak("No file loaded");
                            }
                        }
                    });
                });

                ui.add_space(4.0);
                self.show_tab_bar(ui);
            });
        });

        if self.settings.show_outline {
            let outline = self
                .active_document()
                .map(|doc| doc.outline.clone())
                .unwrap_or_default();
            let mut jump_to_line = None::<usize>;

            egui::SidePanel::left("outline_panel")
                .resizable(true)
                .default_width(220.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Outline");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("×").on_hover_text("Hide outline").clicked() {
                                self.settings.show_outline = false;
                            }
                        });
                    });
                    ui.separator();

                    if ui.button("Top").clicked() {
                        jump_to_line = Some(0);
                    }
                    ui.add_space(4.0);

                    if outline.is_empty() {
                        ui.weak("No headings");
                        return;
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for item in &outline {
                            ui.horizontal(|ui| {
                                ui.add_space(((item.level.saturating_sub(1)) as f32) * 12.0);
                                if ui
                                    .selectable_label(false, &item.title)
                                    .on_hover_text(format!(
                                        "Line {}\n#{}",
                                        item.line + 1,
                                        item.slug
                                    ))
                                    .clicked()
                                {
                                    jump_to_line = Some(item.line);
                                }
                            });
                        }
                    });
                });

            if let Some(line) = jump_to_line {
                if let Some(doc) = self.active_document_mut() {
                    doc.scroll_to_line = Some(line);
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, err);
                ui.separator();
            }

            let math_cache = self.math_cache.clone();
            let math_tx = self.math_tx.clone();
            let mermaid_cache = self.mermaid_cache.clone();
            let mermaid_tx = self.mermaid_tx.clone();
            let render_math_enabled = self.settings.render_math;
            let render_mermaid_enabled = self.settings.render_mermaid;

            if self.documents.get(self.active_doc).is_none() {
                ui.weak("No documents open");
                return;
            }

            let editor_has_focus;
            {
                let doc = &mut self.documents[self.active_doc];
                let edit_resp = ui.add(
                    egui::TextEdit::multiline(&mut doc.raw_markdown)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(12),
                );
                editor_has_focus = edit_resp.has_focus();
                if edit_resp.changed() {
                    doc.rebuild_markdown(&self.settings);
                }
            }
            self.editor_has_focus = editor_has_focus;

            if self.editor_has_focus
                && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V))
            {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(image) = clipboard.get_image() {
                        let bytes = image.bytes.into_owned();
                        self.process_images_for_active_doc(vec![PendingImage {
                            name: "pasted-image.png".to_string(),
                            mime: "image/png".to_string(),
                            bytes,
                        }]);
                    }
                }
            }

            ui.separator();
            ui.label("Preview");

            let line_height = ui.text_style_height(&TextStyle::Body) + ui.spacing().item_spacing.y;
            let (doc_id, scroll_to_line) = {
                let doc = &mut self.documents[self.active_doc];
                (doc.id, doc.scroll_to_line.take())
            };
            let mut scroll_area = egui::ScrollArea::vertical().id_salt(doc_id);
            if let Some(line) = scroll_to_line {
                scroll_area = scroll_area.vertical_scroll_offset((line as f32) * line_height);
            }

            scroll_area.show(ui, |ui| {
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

                let doc = &mut self.documents[self.active_doc];
                viewer.show(ui, &mut doc.commonmark_cache, &doc.markdown);
            });

            if self.drop_zone_visible && self.editor_has_focus {
                let rect = ui.max_rect();
                let painter = ui.painter();
                painter.rect_filled(
                    rect,
                    egui::CornerRadius::same(8),
                    egui::Color32::from_rgba_unmultiplied(59, 130, 246, 20),
                );
                painter.rect_stroke(
                    rect.shrink(4.0),
                    egui::CornerRadius::same(8),
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(59, 130, 246)),
                    egui::StrokeKind::Middle,
                );
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🖼 Drop image here",
                    egui::TextStyle::Heading.resolve(ui.style()),
                    egui::Color32::from_rgb(59, 130, 246),
                );
            }
        });

        self.show_find_window(ctx);
        self.handle_internal_anchor_links(ctx);
        self.show_notifications(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.persisted.settings = self.settings.clone();
        self.persisted.open_files = self
            .documents
            .iter()
            .filter_map(|doc| doc.file_path.clone())
            .collect();
        self.persisted.active_file = self.active_document().and_then(|doc| doc.file_path.clone());
        eframe::set_value(storage, STATE_KEY, &self.persisted);
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn infer_mime_from_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppTheme {
    System,
    Dark,
    Light,
    TerminalGreen,
    TerminalAmber,
}

impl Default for AppTheme {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, Copy)]
enum TerminalHue {
    Green,
    Amber,
}

fn restore_default_visuals(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::dark());
    ctx.set_visuals_of(egui::Theme::Light, egui::Visuals::light());
}

fn apply_app_theme(ctx: &egui::Context, theme: AppTheme) {
    match theme {
        AppTheme::System => {
            restore_default_visuals(ctx);
            ctx.set_theme(egui::ThemePreference::System);
        }
        AppTheme::Dark => {
            restore_default_visuals(ctx);
            ctx.set_theme(egui::ThemePreference::Dark);
        }
        AppTheme::Light => {
            restore_default_visuals(ctx);
            ctx.set_theme(egui::ThemePreference::Light);
        }
        AppTheme::TerminalGreen => {
            ctx.set_theme(egui::ThemePreference::Dark);
            ctx.set_visuals_of(egui::Theme::Dark, terminal_visuals(TerminalHue::Green));
        }
        AppTheme::TerminalAmber => {
            ctx.set_theme(egui::ThemePreference::Dark);
            ctx.set_visuals_of(egui::Theme::Dark, terminal_visuals(TerminalHue::Amber));
        }
    }
}

fn terminal_visuals(hue: TerminalHue) -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();

    let bg = egui::Color32::from_rgb(0x07, 0x0b, 0x07);
    let extreme_bg = egui::Color32::from_rgb(0x04, 0x06, 0x04);

    let (accent, accent_dim, accent_bright) = match hue {
        TerminalHue::Green => (
            egui::Color32::from_rgb(0x00, 0xff, 0x7a),
            egui::Color32::from_rgb(0x00, 0xc8, 0x60),
            egui::Color32::from_rgb(0xc8, 0xff, 0xe8),
        ),
        TerminalHue::Amber => (
            egui::Color32::from_rgb(0xff, 0xb0, 0x36),
            egui::Color32::from_rgb(0xe6, 0x9a, 0x20),
            egui::Color32::from_rgb(0xff, 0xf0, 0xd0),
        ),
    };

    let button_bg = lerp_color(bg, accent, 0.12);
    let hovered_bg = lerp_color(bg, accent, 0.18);
    let active_bg = lerp_color(bg, accent, 0.24);

    visuals.window_fill = bg;
    visuals.panel_fill = bg;
    visuals.window_stroke = egui::Stroke::new(1.0, with_alpha(accent_dim, 140));
    visuals.faint_bg_color = lerp_color(bg, accent_dim, 0.06);
    visuals.extreme_bg_color = extreme_bg;
    visuals.text_edit_bg_color = Some(extreme_bg);
    visuals.code_bg_color = lerp_color(bg, accent, 0.08);
    visuals.hyperlink_color = egui::Color32::from_rgb(0x66, 0xd9, 0xef); // cyan
    visuals.selection.bg_fill = lerp_color(bg, accent, 0.35);
    visuals.selection.stroke = egui::Stroke::new(1.0, accent_bright);

    visuals.widgets.noninteractive.bg_fill = bg;
    visuals.widgets.noninteractive.weak_bg_fill = bg;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, with_alpha(accent_dim, 140));
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, accent);

    visuals.widgets.inactive.weak_bg_fill = button_bg;
    visuals.widgets.inactive.bg_fill = button_bg;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, with_alpha(accent_dim, 110));
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, accent);

    visuals.widgets.hovered.weak_bg_fill = hovered_bg;
    visuals.widgets.hovered.bg_fill = hovered_bg;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, with_alpha(accent_bright, 200));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, accent_bright);

    visuals.widgets.active.weak_bg_fill = active_bg;
    visuals.widgets.active.bg_fill = active_bg;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent_bright);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, accent_bright);

    visuals.widgets.open.weak_bg_fill = hovered_bg;
    visuals.widgets.open.bg_fill = bg;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, with_alpha(accent_dim, 140));
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, accent_bright);

    visuals.warn_fg_color = egui::Color32::from_rgb(0xff, 0xb0, 0x36);

    visuals
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ViewerSettings {
    theme: AppTheme,
    render_math: bool,
    render_mermaid: bool,
    auto_detect_code_lang: bool,
    autolink_urls: bool,
    github_links: bool,
    replace_emoji: bool,
    smart_typography: bool,
    show_outline: bool,
    auto_reload: bool,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            theme: AppTheme::System,
            render_math: true,
            render_mermaid: true,
            auto_detect_code_lang: true,
            autolink_urls: true,
            github_links: true,
            replace_emoji: true,
            smart_typography: false,
            show_outline: true,
            auto_reload: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PersistedState {
    settings: ViewerSettings,
    recent_files: Vec<PathBuf>,
    open_files: Vec<PathBuf>,
    active_file: Option<PathBuf>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            settings: ViewerSettings::default(),
            recent_files: Vec::new(),
            open_files: Vec::new(),
            active_file: None,
        }
    }
}

impl PersistedState {
    fn remember_file(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        if self.recent_files.len() > MAX_RECENT_FILES {
            self.recent_files.truncate(MAX_RECENT_FILES);
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
    let lang = lang
        .trim_matches(|c| c == '{' || c == '}' || c == '.')
        .strip_prefix("language-")
        .unwrap_or(lang)
        .trim();
    let lang_lc = lang.to_ascii_lowercase();
    let mapped = match lang_lc.as_str() {
        // Core languages
        "rust" => "rs",
        "python" => "py",
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "bash" | "sh" | "shell" => "sh",
        "zsh" | "fish" => "sh",
        "powershell" | "pwsh" => "ps1",
        "csharp" | "cs" => "cs",
        "cpp" | "c++" => "cpp",
        "c" => "c",
        "h" => "h",
        "h++" | "hpp" => "hpp",
        "go" | "golang" => "go",
        "java" => "java",
        "kotlin" | "kt" | "kts" => "kt",
        "swift" => "swift",
        "scala" => "scala",
        "ruby" | "rb" => "rb",
        "php" => "php",
        "perl" | "pl" => "pl",
        "lua" => "lua",
        "r" => "r",
        "julia" | "jl" => "jl",
        "dart" => "dart",
        "elixir" | "ex" | "exs" => "ex",
        "erlang" | "erl" => "erl",
        "haskell" | "hs" => "hs",
        "ocaml" | "ml" => "ml",
        "nim" => "nim",
        "zig" => "zig",
        "solidity" | "sol" => "sol",
        // Web + data formats
        "json" => "json",
        "json5" => "json5",
        "toml" => "toml",
        "ini" => "ini",
        "yaml" | "yml" => "yml",
        "markdown" | "md" => "md",
        "graphql" | "gql" => "graphql",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "html" => "html",
        "xml" => "xml",
        "svg" => "svg",
        "csv" => "csv",
        "sql" => "sql",
        "proto" | "protobuf" => "proto",
        // Tooling / config
        "docker" | "dockerfile" => "dockerfile",
        "make" | "makefile" => "makefile",
        "cmake" => "cmake",
        "nix" => "nix",
        "diff" | "patch" => "diff",
        "gitignore" => "gitignore",
        "editorconfig" => "editorconfig",
        "tex" | "latex" => "tex",
        "plaintext" | "text" | "txt" => "txt",
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
