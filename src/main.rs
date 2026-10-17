use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "hwp2md")]
#[command(about = "HWP/HWPX ↔ Markdown bidirectional converter")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert HWP/HWPX to Markdown
    ToMd {
        /// Input HWP or HWPX file
        input: PathBuf,
        /// Output Markdown file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Extract images to this directory
        #[arg(long)]
        assets_dir: Option<PathBuf>,
        /// Include frontmatter metadata
        #[arg(long)]
        frontmatter: bool,
        /// Table rendering format for merged cells (colspan/rowspan > 1)
        #[arg(long, value_enum, default_value = "html")]
        table_format: hwp2md::convert::TableFormat,
    },
    /// Convert Markdown to HWPX
    ToHwpx {
        /// Input Markdown file
        input: PathBuf,
        /// Output HWPX file
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Style template (YAML)
        #[arg(long)]
        style: Option<PathBuf>,
    },
    /// Show document info without converting
    Info {
        /// Input HWP or HWPX file
        input: PathBuf,
    },
    /// Validate a file by parsing it without writing output
    Check {
        /// Input file (.hwp, .hwpx, .md, or .markdown)
        input: PathBuf,
    },
    /// Convert between supported formats by inferring the direction from
    /// the input and output file extensions.
    ///
    /// Supported pairs:
    ///   .hwp / .hwpx  →  .md / .markdown
    ///   .md / .markdown  →  .hwpx
    Convert {
        /// Input file (.hwp, .hwpx, .md, or .markdown)
        input: PathBuf,
        /// Output file (.md, .markdown, or .hwpx)
        output: PathBuf,
        /// Overwrite the output file if it already exists
        #[arg(long)]
        force: bool,
        /// Extract images to this directory (HWP/HWPX → MD only)
        #[arg(long)]
        assets_dir: Option<PathBuf>,
        /// Include frontmatter metadata (HWP/HWPX → MD only)
        #[arg(long)]
        frontmatter: bool,
        /// Style template YAML (MD → HWPX only)
        #[arg(long)]
        style: Option<PathBuf>,
    },
    /// Batch-convert all HWP/HWPX files in a directory (recursively) to Markdown,
    /// mirroring the subdirectory structure in the output directory
    Batch {
        /// Input directory containing .hwp/.hwpx files (searched recursively)
        input_dir: PathBuf,
        /// Output directory for .md files (default: same as input)
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
        /// Include frontmatter metadata
        #[arg(long)]
        frontmatter: bool,
        /// Overwrite existing output files
        #[arg(long)]
        force: bool,
        /// Extract images to per-file subdirectories under this path
        #[arg(long)]
        assets_dir: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .init();

    match cli.command {
        Commands::ToMd {
            input,
            output,
            assets_dir,
            frontmatter,
            table_format,
        } => {
            hwp2md::convert::to_markdown_with_table_format(
                &input,
                output.as_deref(),
                assets_dir.as_deref(),
                frontmatter,
                table_format,
            )?;
        }
        Commands::ToHwpx {
            input,
            output,
            style,
        } => {
            hwp2md::convert::to_hwpx(&input, output.as_deref(), style.as_deref())?;
        }
        Commands::Info { input } => {
            hwp2md::convert::show_info(&input)?;
        }
        Commands::Check { input } => {
            hwp2md::convert::check(&input)?;
            println!("OK: {}", input.display());
        }
        Commands::Convert {
            input,
            output,
            force,
            assets_dir,
            frontmatter,
            style,
        } => {
            let mut opts = hwp2md::convert::ConvertOptions::new(&input, &output)
                .force(force)
                .frontmatter(frontmatter);
            if let Some(ref dir) = assets_dir {
                opts = opts.assets_dir(dir);
            }
            if let Some(ref path) = style {
                opts = opts.style(path);
            }
            opts.execute()?;
        }
        Commands::Batch {
            input_dir,
            output_dir,
            frontmatter,
            force,
            assets_dir,
        } => {
            run_batch(
                &input_dir,
                output_dir.as_deref(),
                frontmatter,
                force,
                assets_dir.as_deref(),
            )?;
        }
    }

    Ok(())
}

/// Batch-convert all `.hwp` / `.hwpx` files under `input_dir` to Markdown.
///
/// Subdirectories are walked recursively and the directory structure is
/// mirrored in `output_dir` (or `input_dir` when not supplied): for example
/// `input/a/b/doc.hwp` becomes `output/a/b/doc.md`.  When `assets_dir` is
/// given, images go to `assets_dir/a/b/doc/`.  Hidden entries and symlinks
/// (files and directories) are skipped.
///
/// Conversion errors for individual files are logged and counted; the
/// function returns `Ok(())` as long as at least one file succeeded (or zero
/// files were found).  It only returns an `Err` when the input directory
/// cannot be read or all files failed.
fn run_batch(
    input_dir: &std::path::Path,
    output_dir: Option<&std::path::Path>,
    frontmatter: bool,
    force: bool,
    assets_dir: Option<&std::path::Path>,
) -> Result<()> {
    if !input_dir.exists() {
        anyhow::bail!("input directory does not exist: {}", input_dir.display());
    }
    if !input_dir.is_dir() {
        anyhow::bail!("input path is not a directory: {}", input_dir.display());
    }

    let out_dir = output_dir.unwrap_or(input_dir);
    if !out_dir.exists() {
        std::fs::create_dir_all(out_dir)?;
    }

    // Collect every input first so files written during conversion (e.g. when
    // the output dir lives inside the input dir) are never picked up.
    let mut inputs = Vec::new();
    collect_hwp_files(input_dir, &mut inputs)?;
    inputs.sort();

    let mut converted: usize = 0;
    let mut skipped: usize = 0;
    let mut failed: usize = 0;

    for path in inputs {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            tracing::warn!("Skipping file with non-UTF-8 stem: {:?}", path);
            failed += 1;
            continue;
        };

        // Directory of this file relative to the input root ("" at top level).
        let rel_dir = path
            .parent()
            .and_then(|p| p.strip_prefix(input_dir).ok())
            .unwrap_or_else(|| std::path::Path::new(""));

        let target_dir = out_dir.join(rel_dir);
        let out_path = target_dir.join(format!("{stem}.md"));

        if !force && out_path.exists() {
            eprintln!(
                "Skipping {:?}: output already exists (use --force to overwrite)",
                path.display()
            );
            skipped += 1;
            continue;
        }

        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            eprintln!("Error creating {}: {e}", target_dir.display());
            failed += 1;
            continue;
        }

        let file_assets_dir = assets_dir.map(|base| base.join(rel_dir).join(stem));

        match hwp2md::convert::to_markdown(
            &path,
            Some(&out_path),
            file_assets_dir.as_deref(),
            frontmatter,
        ) {
            Ok(()) => {
                println!("Converted: {} -> {}", path.display(), out_path.display());
                converted += 1;
            }
            Err(e) => {
                eprintln!("Error converting {}: {e}", path.display());
                failed += 1;
            }
        }
    }

    println!("Batch complete: {converted} converted, {skipped} skipped, {failed} failed");

    if converted == 0 && failed > 0 {
        anyhow::bail!("All {failed} file(s) failed to convert");
    }

    Ok(())
}

/// Recursively collect `.hwp` / `.hwpx` files under `dir`, skipping hidden
/// entries and symlinks.
fn collect_hwp_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if entry.file_name().to_string_lossy().starts_with('.') {
            tracing::debug!("Skipping hidden entry: {:?}", path);
            continue;
        }

        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::debug!("Skipping symlink: {:?}", path);
            continue;
        }

        if file_type.is_dir() {
            if let Err(e) = collect_hwp_files(&path, out) {
                eprintln!("Error reading directory {}: {e}", path.display());
            }
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "hwp" || ext == "hwpx" {
            out.push(path);
        }
    }
    Ok(())
}
