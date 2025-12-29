# markdownviewer

A Windows GUI Markdown file viewer written in Rust (built with `eframe`/`egui`).

## Features

- CommonMark + GitHub Flavored Markdown (tables, task lists, strikethrough)
- Footnotes, definition lists, and GitHub-style callouts/admonitions
- Math rendering (`$...$` / `$$...$$`) via MathJax → SVG
- Mermaid diagrams via Kroki (` ```mermaid ` fences → SVG; requires internet)
- Syntax highlighting + copy buttons (with best-effort language auto-detect)
- Outline panel + in-document Find (Ctrl+F)
- Light/Dark/System theme toggle
- Emoji shortcodes (`:rocket:`) + URL autolinks + GitHub issue/PR links (`#123`, `PR#123`)
- Optional smart typography (off by default)
- Recent files + session restore, optional auto-reload on file changes

## Usage

Run the app:

```bash
cargo run --release
```

Quick demo:

```bash
cargo run --release -- example.md
```

Build a Windows executable:

```bash
cargo build --release
```

Cross-compile a Windows `.exe` from Linux/WSL (requires `cargo-xwin`):

```bash
cargo install cargo-xwin
cargo xwin build --target x86_64-pc-windows-msvc --release
```

Notes:

- Cross-compiling uses `zig` as a `clang-cl` shim via `.cargo/config.toml` + `tools/clang-cl`.

Open a file:

- Click **Open…**
- You can select multiple files to open them as tabs
- Or drag and drop a `.md` file onto the window
- Or pass a path on startup: `markdownviewer path/to/file.md`

Reload the current file with **Reload**.
