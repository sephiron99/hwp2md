/// CLI integration tests — `to-md` subcommand.
///
/// Each test spawns the compiled binary via `std::process::Command` and
/// verifies exit codes and output text.
use tempfile::tempdir;

#[path = "common/mod.rs"]
mod common;

use common::cargo_bin;

// ---------------------------------------------------------------------------
// 4. to-md --help → shows input/output/assets-dir/frontmatter options
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_help() {
    let output = cargo_bin()
        .args(["to-md", "--help"])
        .output()
        .expect("failed to execute hwp2md to-md --help");
    assert!(
        output.status.success(),
        "expected zero exit; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("input") || stdout.contains("INPUT"),
        "input option missing: {stdout}"
    );
    assert!(
        stdout.contains("output") || stdout.contains("OUTPUT"),
        "output option missing: {stdout}"
    );
    assert!(
        stdout.contains("assets-dir") || stdout.contains("assets"),
        "assets-dir option missing: {stdout}"
    );
    assert!(
        stdout.contains("frontmatter"),
        "frontmatter option missing: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 7. to-md nonexistent.hwp → non-zero exit, error about file not found
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_nonexistent_file() {
    let output = cargo_bin()
        .args(["to-md", "/nonexistent/path/file.hwp"])
        .output()
        .expect("failed to execute hwp2md to-md");
    assert!(
        !output.status.success(),
        "expected non-zero exit for missing file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // anyhow wraps OS errors; the message will contain the path or "No such file"
    assert!(
        stderr.contains("nonexistent")
            || stderr.contains("No such file")
            || stderr.contains("not found")
            || stderr.contains("os error"),
        "expected file-not-found error, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 8. to-md invalid.txt → non-zero exit, error about unsupported format
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_unsupported_format() {
    let dir = tempdir().expect("tempdir");
    let txt_file = dir.path().join("document.txt");
    std::fs::write(&txt_file, b"some content").expect("write txt");

    let output = cargo_bin()
        .args(["to-md", txt_file.to_str().unwrap()])
        .output()
        .expect("failed to execute hwp2md to-md");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unsupported extension"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unsupported") || stderr.contains("unsupported"),
        "expected unsupported-format error, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 10. to-md with a real HWPX produced by to-hwpx
//     → exit 0, stdout contains markdown
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_from_hwpx() {
    let dir = tempdir().expect("tempdir");

    // Step 1: create a markdown source file.
    let md_src = dir.path().join("source.md");
    std::fs::write(&md_src, b"# Integration Test\n\nHello from CLI test.\n").expect("write md");

    // Step 2: use `to-hwpx` to produce a valid HWPX.
    let hwpx_path = dir.path().join("output.hwpx");
    let convert_out = cargo_bin()
        .args([
            "to-hwpx",
            md_src.to_str().unwrap(),
            "--output",
            hwpx_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute hwp2md to-hwpx");
    assert!(
        convert_out.status.success(),
        "to-hwpx failed; stderr: {}",
        String::from_utf8_lossy(&convert_out.stderr)
    );
    assert!(hwpx_path.exists(), "HWPX file was not created");

    // Step 3: run `to-md` on the produced HWPX; expect markdown on stdout.
    let md_out = cargo_bin()
        .args(["to-md", hwpx_path.to_str().unwrap()])
        .output()
        .expect("failed to execute hwp2md to-md");
    assert!(
        md_out.status.success(),
        "to-md failed; stderr: {}",
        String::from_utf8_lossy(&md_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&md_out.stdout);
    assert!(
        stdout.contains("Integration Test") || stdout.contains('#'),
        "expected markdown content in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 11. to-md --table-format {html,grid,repeat} on a real HWP fixture → exit 0
//     for all three values; --help documents the flag (Sprint: v0.6.0
//     configurable table format).
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_help_documents_table_format_flag() {
    let output = cargo_bin()
        .args(["to-md", "--help"])
        .output()
        .expect("failed to execute hwp2md to-md --help");
    assert!(
        output.status.success(),
        "expected zero exit; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("table-format"),
        "table-format option missing: {stdout}"
    );
    assert!(
        stdout.contains("html") && stdout.contains("grid") && stdout.contains("repeat"),
        "expected html/grid/repeat possible values documented: {stdout}"
    );
}

#[test]
fn cli_to_md_table_format_grid_accepted_on_real_fixture() {
    let dir = tempdir().expect("tempdir");
    let out_path = dir.path().join("out.md");
    let output = cargo_bin()
        .args([
            "to-md",
            "tests/fixtures/real/moel_01_goyang_center.hwp",
            "--table-format",
            "grid",
            "-o",
        ])
        .arg(&out_path)
        .output()
        .expect("failed to execute hwp2md to-md --table-format grid");
    assert!(
        output.status.success(),
        "expected zero exit; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_path.exists(), "output file was not created");
}

#[test]
fn cli_to_md_table_format_repeat_accepted_on_real_fixture() {
    let dir = tempdir().expect("tempdir");
    let out_path = dir.path().join("out.md");
    let output = cargo_bin()
        .args([
            "to-md",
            "tests/fixtures/real/moel_01_goyang_center.hwp",
            "--table-format",
            "repeat",
            "-o",
        ])
        .arg(&out_path)
        .output()
        .expect("failed to execute hwp2md to-md --table-format repeat");
    assert!(
        output.status.success(),
        "expected zero exit; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_path.exists(), "output file was not created");
}

#[test]
fn cli_to_md_default_table_format_matches_explicit_html() {
    let dir = tempdir().expect("tempdir");
    let default_out = dir.path().join("default.md");
    let explicit_out = dir.path().join("explicit.md");

    let default_status = cargo_bin()
        .args([
            "to-md",
            "tests/fixtures/real/moel_01_goyang_center.hwp",
            "-o",
        ])
        .arg(&default_out)
        .status()
        .expect("failed to execute hwp2md to-md (default)");
    assert!(default_status.success(), "default invocation failed");

    let explicit_status = cargo_bin()
        .args([
            "to-md",
            "tests/fixtures/real/moel_01_goyang_center.hwp",
            "--table-format",
            "html",
            "-o",
        ])
        .arg(&explicit_out)
        .status()
        .expect("failed to execute hwp2md to-md --table-format html");
    assert!(explicit_status.success(), "explicit html invocation failed");

    let default_content = std::fs::read_to_string(&default_out).expect("read default output");
    let explicit_content = std::fs::read_to_string(&explicit_out).expect("read explicit output");
    assert_eq!(
        default_content, explicit_content,
        "omitting --table-format must be byte-identical to --table-format html"
    );
}
