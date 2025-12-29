use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=assets/markdownviewer.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // If cross-compiling to `*-windows-msvc` on a non-Windows host, embed-resource defaults to `llvm-rc`,
    // which requires a C compiler (often `clang-cl`) for preprocessing. If that's not available but `zig`
    // is present, we can emulate GNU windres with `zig rc` to avoid the C preprocessor step.
    let target = env::var("TARGET").unwrap_or_default();
    if target.ends_with("-windows-msvc")
        && !cfg!(target_os = "windows")
        && rc_env_is_unset(&target)
        && !is_in_path("clang-cl")
    {
        if let Some(zig) = which("zig") {
            if let Ok(out_dir) = env::var("OUT_DIR") {
                if let Some(wrapper) = write_zig_windres_wrapper(&PathBuf::from(out_dir), &zig) {
                    unsafe {
                        env::set_var("RC", wrapper);
                    }
                }
            }
        }
    }

    embed_resource::compile("assets/markdownviewer.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}

fn rc_env_is_unset(target: &str) -> bool {
    env::var(format!("RC_{target}")).is_err()
        && env::var(format!("RC_{}", target.replace('-', "_"))).is_err()
        && env::var("RC").is_err()
}

fn is_in_path(exe_name: &str) -> bool {
    which(exe_name).is_some()
}

fn which(exe_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(exe_name))
        .find(|candidate| candidate.is_file())
}

fn write_zig_windres_wrapper(out_dir: &Path, zig: &Path) -> Option<PathBuf> {
    let wrapper_path = out_dir.join("windres");

    // This wrapper is used only on non-Windows hosts, so a bash script is fine.
    // embed-resource drives GNU windres with:
    //   --input <rc> --output-format=coff --output <out> --include-dir <dir> [-D <macro>]*
    // and we translate that to `zig rc` outputting a COFF object.
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${{1:-}}" == "-V" && "${{2:-}}" == "/?" ]]; then
  echo "GNU windres (zig rc wrapper)"
  exit 0
fi

input=""
output=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --input) input="$2"; shift 2;;
    --output) output="$2"; shift 2;;
    --output-format=*) shift;;
    --include-dir) shift 2;;
    -D) shift 2;;
    *) shift;;
  esac
done

if [[ -z "$input" || -z "$output" ]]; then
  echo "windres wrapper: missing --input or --output" >&2
  exit 2
fi

exec "{zig}" rc "/:output-format" "coff" "/:target" "x86_64" "/fo" "$output" -- "$input"
"#,
        zig = zig.display()
    );

    fs::write(&wrapper_path, script).ok()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = fs::metadata(&wrapper_path).ok()?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper_path, perms).ok()?;
    }

    Some(wrapper_path)
}
