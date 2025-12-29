# markdownviewer

A basic Windows GUI Markdown file viewer written in Rust (built with `eframe`/`egui`).

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

Open a file:

- Click **Open…**
- Or drag and drop a `.md` file onto the window
- Or pass a path on startup: `markdownviewer path/to/file.md`

Reload the current file with **Reload**.

## Icon

Replace `assets/icon.ico` and rebuild to change the Windows `.exe` icon.
