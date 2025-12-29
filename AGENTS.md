## Codex instructions (markdownviewer)

- Build artifacts live in `target/` (including cross-compiled outputs).
- Do not copy executables/symbols into `dist/` or any other directory unless the user explicitly asks.
- When asked to “build the exe”, run the appropriate `cargo build`/`cargo xwin build` and report the path under `target/...`.
