/// CLI integration tests — `to-md` subcommand.
///
/// Each test spawns the compiled binary via `std::process::Command` and
/// verifies exit codes and output text.
use tempfile::tempdir;

#[path = "common/mod.rs"]
mod common;

#[path = "fixtures/mod.rs"]
#[allow(dead_code)]
mod fixtures;

use common::cargo_bin;

/// HWPX fixture with a table whose first row is a merged (colspan=2) cell,
/// so every `--table-format` value takes a different rendering path.
fn merged_table_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let table_xml = r#"<hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:tbl rowCnt="2" colCnt="2">
        <hp:tr>
            <hp:tc>
                <hp:cellSpan colSpan="2" rowSpan="1"/>
                <hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>Merged Header</hp:t></hp:run></hp:p>
            </hp:tc>
        </hp:tr>
        <hp:tr>
            <hp:tc><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>Cell A</hp:t></hp:run></hp:p></hp:tc>
            <hp:tc><hp:p paraPrIDRef="0"><hp:run charPrIDRef="0"><hp:t>Cell B</hp:t></hp:run></hp:p></hp:tc>
        </hp:tr>
    </hp:tbl></hp:run></hp:p>"#;
    fixtures::HwpxFixture::new()
        .section(table_xml)
        .write_to_tempfile()
}

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
// 7. to-md nonexistent.hwpx → non-zero exit, error about file not found
// ---------------------------------------------------------------------------

#[test]
fn cli_to_md_nonexistent_file() {
    let output = cargo_bin()
        .args(["to-md", "/nonexistent/path/file.hwpx"])
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
fn cli_to_md_table_format_grid_accepted_on_merged_table() {
    let (_fx_dir, fixture) = merged_table_fixture();
    let dir = tempdir().expect("tempdir");
    let out_path = dir.path().join("out.md");
    let output = cargo_bin()
        .args([
            "to-md",
            fixture.to_str().unwrap(),
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
    let md = std::fs::read_to_string(&out_path).expect("read output");
    assert!(md.contains("+--"), "expected grid table syntax: {md}");
}

#[test]
fn cli_to_md_table_format_repeat_accepted_on_merged_table() {
    let (_fx_dir, fixture) = merged_table_fixture();
    let dir = tempdir().expect("tempdir");
    let out_path = dir.path().join("out.md");
    let output = cargo_bin()
        .args([
            "to-md",
            fixture.to_str().unwrap(),
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
    let (_fx_dir, fixture) = merged_table_fixture();
    let dir = tempdir().expect("tempdir");
    let default_out = dir.path().join("default.md");
    let explicit_out = dir.path().join("explicit.md");

    let default_status = cargo_bin()
        .args(["to-md", fixture.to_str().unwrap(), "-o"])
        .arg(&default_out)
        .status()
        .expect("failed to execute hwp2md to-md (default)");
    assert!(default_status.success(), "default invocation failed");

    let explicit_status = cargo_bin()
        .args([
            "to-md",
            fixture.to_str().unwrap(),
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

#[test]
fn cli_to_md_hwp_input_rejected_with_hint() {
    let dir = tempdir().expect("tempdir");
    let input = dir.path().join("legacy.hwp");
    std::fs::write(&input, b"not parsed").expect("write hwp");
    let output = cargo_bin()
        .args(["to-md", input.to_str().unwrap()])
        .output()
        .expect("failed to execute hwp2md to-md");
    assert!(!output.status.success(), ".hwp input must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HWP 5.0") && stderr.contains(".hwpx"),
        "expected HWPX hint in stderr: {stderr}"
    );
}
