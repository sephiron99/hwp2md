/// CLI integration tests — `batch` subcommand.
///
/// These tests exercise the `batch` directory-conversion command end-to-end by
/// spawning the compiled binary via `std::process::Command`.  A minimal valid
/// HWPX is produced by the `make_hwpx` helper, which calls `to-hwpx` on a
/// temporary Markdown source.
use tempfile::tempdir;

#[path = "common/mod.rs"]
mod common;

use common::{cargo_bin, make_hwpx};

// ---------------------------------------------------------------------------
// 18. batch --help → shows input-dir, output-dir, frontmatter, force options
// ---------------------------------------------------------------------------

#[test]
fn batch_help_shows_options() {
    let output = cargo_bin()
        .args(["batch", "--help"])
        .output()
        .expect("failed to execute hwp2md batch --help");
    assert!(
        output.status.success(),
        "expected zero exit; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("input-dir") || stdout.contains("INPUT_DIR"),
        "input-dir missing: {stdout}"
    );
    assert!(
        stdout.contains("output-dir") || stdout.contains("OUTPUT_DIR"),
        "output-dir missing: {stdout}"
    );
    assert!(
        stdout.contains("frontmatter"),
        "frontmatter flag missing: {stdout}"
    );
    assert!(stdout.contains("force"), "force flag missing: {stdout}");
}

// ---------------------------------------------------------------------------
// 19. batch on an empty directory → exit 0, "0 converted" in stdout
// ---------------------------------------------------------------------------

#[test]
fn batch_empty_directory() {
    let dir = tempdir().expect("tempdir");

    let output = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    assert!(
        output.status.success(),
        "batch on empty dir must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 converted"),
        "expected '0 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 20. batch converts .hwpx files → .md files appear in the output dir
// ---------------------------------------------------------------------------

#[test]
fn batch_converts_hwpx_files() {
    let dir = tempdir().expect("tempdir");

    // Create two valid HWPX fixtures.
    let hwpx1 = dir.path().join("alpha.hwpx");
    let hwpx2 = dir.path().join("beta.hwpx");
    make_hwpx(&hwpx1);
    make_hwpx(&hwpx2);

    let out_dir = dir.path().join("output");

    let result = cargo_bin()
        .args([
            "batch",
            dir.path().to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(
        out_dir.join("alpha.md").exists(),
        "alpha.md not found in output dir"
    );
    assert!(
        out_dir.join("beta.md").exists(),
        "beta.md not found in output dir"
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("2 converted"),
        "expected '2 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 21. batch skips non-.hwp/.hwpx files → .txt files are ignored
// ---------------------------------------------------------------------------

#[test]
fn batch_skips_non_hwp_files() {
    let dir = tempdir().expect("tempdir");

    // Place plain-text files that must be silently ignored.
    std::fs::write(dir.path().join("notes.txt"), "some notes").expect("write txt");
    std::fs::write(dir.path().join("readme.md"), "# Readme").expect("write md");

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0 even with only non-HWP files; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("0 converted"),
        "expected '0 converted' in stdout, got: {stdout}"
    );
    // No .md output for the ignored files.
    assert!(
        !dir.path().join("notes.md").exists(),
        "notes.md must not be created from notes.txt"
    );
}

// ---------------------------------------------------------------------------
// 22. batch on nonexistent directory → non-zero exit, clear error on stderr
// ---------------------------------------------------------------------------

#[test]
fn batch_nonexistent_directory() {
    let result = cargo_bin()
        .args(["batch", "/nonexistent/path/to/dir"])
        .output()
        .expect("execute batch");
    assert!(
        !result.status.success(),
        "batch must exit non-zero for missing directory"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("does not exist")
            || stderr.contains("not found")
            || stderr.contains("No such"),
        "expected 'does not exist' or similar in stderr, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 23. batch on a file path (not a directory) → non-zero exit, clear error
// ---------------------------------------------------------------------------

#[test]
fn batch_input_is_file_not_directory() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("file.hwpx");
    std::fs::write(&file, b"not a dir").expect("write file");

    let result = cargo_bin()
        .args(["batch", file.to_str().unwrap()])
        .output()
        .expect("execute batch");
    assert!(
        !result.status.success(),
        "batch must exit non-zero when input is a file"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("not a directory") || stderr.contains("not a dir"),
        "expected 'not a directory' in stderr, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// 24. batch without --force skips already-existing output files
// ---------------------------------------------------------------------------

#[test]
fn batch_skips_existing_output_without_force() {
    let dir = tempdir().expect("tempdir");

    let hwpx = dir.path().join("doc.hwpx");
    make_hwpx(&hwpx);

    // Pre-create the would-be output file.
    let out_md = dir.path().join("doc.md");
    std::fs::write(&out_md, "existing content").expect("pre-create md");

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    // Should still exit 0 (partial success / zero converted is fine when
    // the only "failure" was an overwrite guard).
    let stdout = String::from_utf8_lossy(&result.stdout);
    let content = std::fs::read_to_string(&out_md).expect("read md");
    assert_eq!(
        content, "existing content",
        "existing output must not be overwritten without --force"
    );
    assert!(
        stdout.contains("1 skipped"),
        "expected '1 skipped' in stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("0 failed"),
        "expected '0 failed' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 25. batch with --force overwrites existing output files
// ---------------------------------------------------------------------------

#[test]
fn batch_overwrites_with_force() {
    let dir = tempdir().expect("tempdir");

    let hwpx = dir.path().join("doc.hwpx");
    make_hwpx(&hwpx);

    // Pre-create the would-be output file with sentinel content.
    let out_md = dir.path().join("doc.md");
    std::fs::write(&out_md, "old content").expect("pre-create md");

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap(), "--force"])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch --force must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let content = std::fs::read_to_string(&out_md).expect("read md");
    assert_ne!(
        content, "old content",
        "output must be overwritten with --force"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("1 converted"),
        "expected '1 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 26. batch skips hidden (dot-prefixed) files
// ---------------------------------------------------------------------------

#[test]
fn batch_skips_hidden_files() {
    let dir = tempdir().expect("tempdir");

    make_hwpx(&dir.path().join("visible.hwpx"));
    make_hwpx(&dir.path().join(".hidden.hwpx"));

    let out_dir = dir.path().join("output");

    let result = cargo_bin()
        .args([
            "batch",
            dir.path().to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(
        out_dir.join("visible.md").exists(),
        "visible.md must be created"
    );
    assert!(
        !out_dir.join(".hidden.md").exists(),
        ".hidden.md must not be created"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("1 converted"),
        "expected '1 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// 27. batch skips symlinks
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn batch_skips_symlinks() {
    let dir = tempdir().expect("tempdir");

    let real = dir.path().join("real.hwpx");
    make_hwpx(&real);

    let link = dir.path().join("link.hwpx");
    std::os::unix::fs::symlink(&real, &link).expect("create symlink");

    let out_dir = dir.path().join("output");

    let result = cargo_bin()
        .args([
            "batch",
            dir.path().to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(out_dir.join("real.md").exists(), "real.md must be created");
    assert!(
        !out_dir.join("link.md").exists(),
        "link.md must not be created (symlink)"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("1 converted"),
        "expected '1 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Sprint 13 — batch --assets-dir flag
// ---------------------------------------------------------------------------

#[test]
fn batch_assets_dir_flag_accepted() {
    let dir = tempdir().expect("tempdir");
    let hwpx = dir.path().join("doc.hwpx");
    common::make_hwpx(&hwpx);
    let assets = dir.path().join("img");

    let result = cargo_bin()
        .args([
            "batch",
            dir.path().to_str().unwrap(),
            "--assets-dir",
            assets.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch --assets-dir");
    assert!(
        result.status.success(),
        "batch --assets-dir failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("1 converted"),
        "expected '1 converted': {stdout}"
    );
}

// ---------------------------------------------------------------------------
// batch recurses into subdirectories and mirrors the structure in output
// ---------------------------------------------------------------------------

#[test]
fn batch_recurses_and_mirrors_structure() {
    let dir = tempdir().expect("tempdir");
    let input = dir.path().join("in");
    std::fs::create_dir_all(input.join("a/b")).expect("mkdir a/b");
    std::fs::create_dir_all(input.join(".hidden")).expect("mkdir .hidden");

    make_hwpx(&input.join("top.hwpx"));
    make_hwpx(&input.join("a/mid.hwpx"));
    make_hwpx(&input.join("a/b/deep.hwpx"));
    make_hwpx(&input.join(".hidden/secret.hwpx"));

    let out_dir = dir.path().join("out");
    let assets = dir.path().join("img");

    let result = cargo_bin()
        .args([
            "batch",
            input.to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
            "--assets-dir",
            assets.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(out_dir.join("top.md").exists(), "top.md missing");
    assert!(out_dir.join("a/mid.md").exists(), "a/mid.md missing");
    assert!(out_dir.join("a/b/deep.md").exists(), "a/b/deep.md missing");
    assert!(
        !out_dir.join(".hidden").exists(),
        "hidden directory must be skipped"
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("3 converted"),
        "expected '3 converted' in stdout, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// batch without --output-dir writes next to each source file
// ---------------------------------------------------------------------------

#[test]
fn batch_recursive_in_place() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir sub");
    make_hwpx(&dir.path().join("sub/doc.hwpx"));

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "batch must exit 0; stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        dir.path().join("sub/doc.md").exists(),
        "sub/doc.md must be created next to its source"
    );
}

// ---------------------------------------------------------------------------
// batch with doc.hwpx + doc.HWPX in one dir → only one writes doc.md
// ---------------------------------------------------------------------------

#[test]
fn batch_same_stem_converted_once() {
    let dir = tempdir().expect("tempdir");
    make_hwpx(&dir.path().join("doc.hwpx"));
    if dir.path().join("doc.HWPX").exists() {
        return; // case-insensitive filesystem: the collision cannot occur
    }
    std::fs::copy(dir.path().join("doc.hwpx"), dir.path().join("doc.HWPX")).expect("copy");

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("another input already maps to"),
        "expected duplicate-output skip; stderr: {stderr}"
    );
    assert!(
        stdout.contains("1 converted") && stdout.contains("1 skipped"),
        "duplicate must be skipped, not attempted; stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// batch reports legacy .hwp files as skipped (HWP 5.0 is not supported)
// ---------------------------------------------------------------------------

#[test]
fn batch_reports_legacy_hwp_as_skipped() {
    let dir = tempdir().expect("tempdir");
    make_hwpx(&dir.path().join("new.hwpx"));
    std::fs::write(dir.path().join("old.hwp"), b"not parsed").expect("write hwp");

    let result = cargo_bin()
        .args(["batch", dir.path().to_str().unwrap()])
        .output()
        .expect("execute batch");
    assert!(result.status.success());
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("old.hwp") && stderr.contains("not supported"),
        "expected legacy .hwp notice; stderr: {stderr}"
    );
    assert!(
        stdout.contains("1 converted, 1 skipped, 0 failed"),
        "unexpected summary: {stdout}"
    );
    assert!(!dir.path().join("old.md").exists());
}

// ---------------------------------------------------------------------------
// batch converts many files in parallel → all outputs present
// ---------------------------------------------------------------------------

#[test]
fn batch_parallel_many_files() {
    let dir = tempdir().expect("tempdir");
    let input = dir.path().join("in");
    let src = dir.path().join("seed.hwpx");
    make_hwpx(&src);
    for i in 0..16 {
        let sub = input.join(format!("d{}", i % 4));
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::copy(&src, sub.join(format!("f{i}.hwpx"))).expect("copy");
    }
    let out_dir = dir.path().join("out");

    let result = cargo_bin()
        .args([
            "batch",
            input.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute batch");
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    for i in 0..16 {
        let md = out_dir.join(format!("d{}", i % 4)).join(format!("f{i}.md"));
        assert!(md.exists(), "{} missing", md.display());
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("16 converted"), "stdout: {stdout}");
}
