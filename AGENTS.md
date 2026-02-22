## Codex instructions (markdownviewer)

- Build artifacts live in `target/` (including cross-compiled outputs).
- Do not copy executables/symbols into `dist/` or any other directory unless the user explicitly asks.
- When asked to “build the exe”, run the appropriate `cargo build`/`cargo xwin build` and report the path under `target/...`.
- Every feature or bugfix code change must include a version bump in `Cargo.toml` (`[package].version`) using SemVer:
  - Bugfix-only changes: bump patch (`x.y.Z`)
  - New backward-compatible features/UI changes: bump minor (`x.Y.0`)
  - Breaking changes: bump major (`X.0.0`)
- Keep `Cargo.lock` in sync with the bumped package version before finishing (run `cargo build` if needed).
