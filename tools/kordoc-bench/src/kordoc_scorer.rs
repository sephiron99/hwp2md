//! kordoc's own Markdown scorer (`bench/compare-md-parsers.mjs`), run on
//! both converters' output as a second opinion next to ours.

use crate::{Doc, Run};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const DRIVER: &str = include_str!("kordoc-score.mjs");

/// `summarize()` of compare-md-parsers.mjs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub docs: usize,
    pub failed: usize,
    pub recall: f64,
    pub recall_parsed: f64,
    pub phantom: f64,
    pub order: f64,
    pub full_recall_docs: usize,
    pub tables: usize,
    pub table_exact: usize,
    pub table_exact_rate: f64,
    pub cell_f1: f64,
}

/// `scoreMd()` of compare-md-parsers.mjs for one document.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocScore {
    pub recall: f64,
    pub phantom_chars: usize,
    pub md_chars: usize,
    pub order: f64,
    pub tables: usize,
    pub table_exact: usize,
    pub cell_f1: f64,
}

#[derive(Debug, Deserialize)]
struct Row {
    idx: String,
    hwp2md: DocScore,
    kordoc: DocScore,
}

#[derive(Debug, Deserialize)]
struct Output {
    summary: HashMap<String, Summary>,
    rows: Vec<Row>,
}

pub struct KordocScores {
    /// Indexed like [`crate::TOOLS`].
    pub summary: [Summary; 2],
    /// Per document index, per tool.
    pub rows: HashMap<String, [DocScore; 2]>,
}

/// Run kordoc's scorer; `Ok(None)` when the checkout has no bench scripts.
pub fn run(
    node: &str,
    kordoc: &Path,
    out: &Path,
    docs: &[Doc],
    runs: [&Vec<Run>; 2],
) -> Result<Option<KordocScores>> {
    if !kordoc.join("bench/compare-md-parsers.mjs").exists() {
        eprintln!(
            "kordoc scorer skipped: {} has no bench/compare-md-parsers.mjs",
            kordoc.display()
        );
        return Ok(None);
    }
    let driver = out.join("kordoc-score.mjs");
    std::fs::write(&driver, DRIVER)?;

    let jobs = out.join("kordoc-score-jobs.tsv");
    let mut f = std::fs::File::create(&jobs)?;
    for (i, d) in docs.iter().enumerate() {
        let md = |tool: &str, run: &Run| {
            if run.ok {
                out.join(tool)
                    .join(format!("{}.md", d.idx))
                    .display()
                    .to_string()
            } else {
                "-".to_string()
            }
        };
        let encrypted = runs[1][i]
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("ENCRYPTED"));
        writeln!(
            f,
            "{}\t{}\t{}\t{}\t{}",
            d.idx,
            d.path.display(),
            md("hwp2md", &runs[0][i]),
            md("kordoc", &runs[1][i]),
            u8::from(encrypted)
        )?;
    }
    drop(f);

    eprintln!("kordoc scorer…");
    let output = Command::new(node)
        .arg(&driver)
        .arg(kordoc)
        .arg(&jobs)
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("start {node}"))?;
    std::fs::write(out.join("kordoc-score-stderr.log"), &output.stderr)?;
    if !output.status.success() {
        bail!(
            "kordoc scorer failed ({}); see kordoc-score-stderr.log",
            output.status
        );
    }
    std::fs::write(out.join("kordoc-score.json"), &output.stdout)?;
    let mut parsed: Output =
        serde_json::from_slice(&output.stdout).context("parse kordoc scorer output")?;
    let mut take = |tool: &str| {
        parsed
            .summary
            .remove(tool)
            .with_context(|| format!("kordoc scorer summary has no {tool}"))
    };
    let summary = [take("hwp2md")?, take("kordoc")?];
    let rows = parsed
        .rows
        .into_iter()
        .map(|r| (r.idx, [r.hwp2md, r.kordoc]))
        .collect();
    Ok(Some(KordocScores { summary, rows }))
}
