//! Benchmark hwp2md against kordoc on HWPX → Markdown.
//!
//! Both converters run on the same sample of a local HWPX corpus.  Their
//! Markdown is scored against text read straight from the HWPX XML (see
//! `reference`), and per-document conversion time is measured in-process for
//! both (kordoc inside one long-lived Node process).  kordoc's own Markdown
//! scorer is run on both outputs too, as a second opinion.

mod kordoc_scorer;
mod mdtext;
mod reference;
mod report;
mod score;
mod visible;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::io::{BufRead, BufReader, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const DRIVER: &str = include_str!("kordoc-driver.mjs");

#[derive(Parser)]
#[command(about = "Compare hwp2md and kordoc HWPX → Markdown output on a local corpus")]
struct Args {
    /// HWPX corpus directory (searched recursively)
    #[arg(long, required_unless_present = "dump_ref")]
    corpus: Option<PathBuf>,
    /// Number of documents to sample (0 = all)
    #[arg(long, default_value_t = 100)]
    sample: usize,
    /// Seed for the sample
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Output directory [default: <workspace>/target/kordoc-bench]
    #[arg(long)]
    out: Option<PathBuf>,
    /// kordoc checkout with a built dist/ [default: ~/prj/kordoc]
    #[arg(long)]
    kordoc: Option<PathBuf>,
    /// Node.js executable
    #[arg(long, default_value = "node")]
    node: String,
    /// Do not convert the first document once before timing
    #[arg(long)]
    no_warmup: bool,
    /// Time conversions straight from the corpus instead of a local copy
    #[arg(long)]
    no_stage: bool,
    /// Skip kordoc's own scorer (bench/compare-md-parsers.mjs)
    #[arg(long)]
    no_kordoc_scorer: bool,
    /// Print the reference paragraphs and tables of one HWPX file and exit
    #[arg(long, value_name = "FILE")]
    dump_ref: Option<PathBuf>,
    /// With --dump-ref: score this Markdown file against it instead
    #[arg(long, value_name = "FILE", requires = "dump_ref")]
    md: Option<PathBuf>,
}

pub struct Doc {
    pub idx: String,
    pub path: PathBuf,
    pub rel: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub ok: bool,
    pub ms: f64,
    pub error: Option<String>,
}

pub const TOOLS: [&str; 2] = ["hwp2md", "kordoc"];

fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(file) = &args.dump_ref {
        let r = reference::extract(file)?;
        if let Some(md_path) = &args.md {
            let md = std::fs::read_to_string(md_path)?;
            let s = score::score(&r, &mdtext::analyze(&md));
            let (t, m) = s.main_chars();
            println!(
                "글 재현율 {m}/{t}, 잡음 {}/{}",
                s.md_chars - s.md_covered,
                s.md_chars
            );
            for (kind, unit, matched, total) in &s.missed {
                println!("[{} {matched}/{total}] {unit}", kind.label());
            }
            for run in &s.noise_runs {
                println!("[잡음 {}자] {run}", run.chars().count());
            }
            return Ok(());
        }
        for u in &r.units {
            println!("[{}] {}", u.kind.label(), u.text);
        }
        for t in &r.tables {
            println!("[표] {}×{}", t.rows, t.cols);
        }
        return Ok(());
    }
    let corpus = args.corpus.clone().expect("clap requires --corpus");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| workspace.join("target/kordoc-bench"));
    let kordoc = match &args.kordoc {
        Some(p) => p.clone(),
        None => PathBuf::from(std::env::var("HOME").context("HOME not set")?).join("prj/kordoc"),
    };
    let kordoc_index = kordoc.join("dist/index.js");
    if !kordoc_index.exists() {
        bail!(
            "{} not found — build kordoc first: (cd {} && npm ci && npm run build)",
            kordoc_index.display(),
            kordoc.display()
        );
    }

    let all = collect_hwpx(&corpus)?;
    if all.is_empty() {
        bail!("no .hwpx files under {}", corpus.display());
    }
    let corpus_total = all.len();
    let mut docs = sample(&corpus, all, args.sample, args.seed);
    eprintln!(
        "{} of {corpus_total} documents sampled (seed {})",
        docs.len(),
        args.seed
    );

    // Outputs are overwritten per index; nothing is deleted.
    for tool in TOOLS {
        std::fs::create_dir_all(out.join(tool))?;
    }
    if !args.no_stage {
        // Corpora on slow mounts (e.g. /mnt/c under WSL) would make file
        // reads dominate the timings, so convert from a local copy.
        let input = out.join("input");
        std::fs::create_dir_all(&input)?;
        for d in &mut docs {
            let local = input.join(format!("{}.hwpx", d.idx));
            std::fs::copy(&d.path, &local).with_context(|| format!("copy {}", d.path.display()))?;
            d.path = local;
        }
    }
    let list = out.join("files.tsv");
    let mut f = std::fs::File::create(&list)?;
    for d in &docs {
        // idx, path converted (local copy), original path in the corpus
        writeln!(f, "{}\t{}\t{}", d.idx, d.path.display(), d.rel)?;
    }
    drop(f);

    let warmup = !args.no_warmup;
    let hwp2md_runs = run_hwp2md(&docs, &out.join("hwp2md"), warmup);
    let kordoc_runs = run_kordoc(&args.node, &kordoc_index, &list, &out, &docs, warmup)?;

    eprintln!("scoring…");
    let results = report::score_all(&docs, &out, [&hwp2md_runs, &kordoc_runs]);
    let kordoc_scores = if args.no_kordoc_scorer {
        None
    } else {
        kordoc_scorer::run(
            &args.node,
            &kordoc,
            &out,
            &docs,
            [&hwp2md_runs, &kordoc_runs],
        )
        .unwrap_or_else(|e| {
            eprintln!("kordoc scorer skipped: {e:#}");
            None
        })
    };
    let info = report::Info {
        corpus: corpus.display().to_string(),
        corpus_total,
        seed: args.seed,
        hwp2md_rev: git_rev(&workspace),
        kordoc_version: kordoc_version(&kordoc),
        kordoc_rev: git_rev(&kordoc),
        warmup,
    };
    let md = report::write(&out, &info, &docs, &results, kordoc_scores.as_ref())?;
    println!("{md}");
    eprintln!("report: {}", out.join("report.md").display());
    Ok(())
}

fn collect_hwpx(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let ty = entry.file_type()?;
            let path = entry.path();
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("hwpx"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Deterministic sample: seeded Fisher–Yates (splitmix64), then path order.
fn sample(root: &Path, mut all: Vec<PathBuf>, n: usize, seed: u64) -> Vec<Doc> {
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    for i in (1..all.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        all.swap(i, j);
    }
    if n > 0 {
        all.truncate(n);
    }
    all.sort();
    all.into_iter()
        .enumerate()
        .map(|(i, path)| Doc {
            idx: format!("{i:04}"),
            rel: path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            path,
        })
        .collect()
}

fn progress(tool: &str, i: usize, n: usize) {
    eprint!("\r[{tool}] {}/{n}", i + 1);
    if i + 1 == n {
        eprintln!();
    }
}

fn run_hwp2md(docs: &[Doc], dir: &Path, warmup: bool) -> Vec<Run> {
    let convert = |src: &Path, dst: &Path| -> Run {
        let t0 = Instant::now();
        let res = panic::catch_unwind(AssertUnwindSafe(|| {
            hwp2md::convert::to_markdown(src, Some(dst), None, false)
        }));
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let error = match res {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(payload) => Some(format!(
                "panic: {}",
                payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("?")
            )),
        };
        Run {
            ok: error.is_none(),
            ms,
            error,
        }
    };

    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    if warmup {
        if let Some(d) = docs.first() {
            convert(&d.path, &dir.join("_warmup.md"));
        }
    }
    let runs = docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let run = convert(&d.path, &dir.join(format!("{}.md", d.idx)));
            progress("hwp2md", i, docs.len());
            run
        })
        .collect();
    panic::set_hook(hook);
    runs
}

#[derive(serde::Deserialize)]
struct DriverRecord {
    idx: String,
    ok: bool,
    ms: f64,
    error: Option<String>,
}

fn run_kordoc(
    node: &str,
    index_js: &Path,
    list: &Path,
    out: &Path,
    docs: &[Doc],
    warmup: bool,
) -> Result<Vec<Run>> {
    let driver = out.join("kordoc-driver.mjs");
    std::fs::write(&driver, DRIVER)?;
    let log = std::fs::File::create(out.join("kordoc-stderr.log"))?;
    let mut child = Command::new(node)
        .arg(&driver)
        .arg(index_js)
        .arg(list)
        .arg(out.join("kordoc"))
        .arg(if warmup { "1" } else { "0" })
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .with_context(|| format!("start {node}"))?;

    let mut runs: Vec<Option<Run>> = vec![None; docs.len()];
    let stdout = child.stdout.take().expect("piped stdout");
    let mut seen = 0;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        // kordoc itself may print to stdout; only driver records are JSON.
        let Ok(rec) = serde_json::from_str::<DriverRecord>(&line) else {
            continue;
        };
        if let Some(i) = docs.iter().position(|d| d.idx == rec.idx) {
            runs[i] = Some(Run {
                ok: rec.ok,
                ms: rec.ms,
                error: rec.error,
            });
            progress("kordoc", seen, docs.len());
            seen += 1;
        }
    }
    let status = child.wait()?;
    if seen < docs.len() {
        eprintln!(
            "\nkordoc driver stopped after {seen}/{} documents ({status}); see kordoc-stderr.log",
            docs.len()
        );
    }
    Ok(runs
        .into_iter()
        .map(|r| {
            r.unwrap_or(Run {
                ok: false,
                ms: 0.0,
                error: Some("kordoc driver exited before this document".into()),
            })
        })
        .collect())
}

fn git_rev(dir: &Path) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "?".into())
}

fn kordoc_version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string))
        .unwrap_or_else(|| "?".into())
}
