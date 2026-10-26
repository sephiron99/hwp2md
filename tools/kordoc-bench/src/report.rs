//! Scoring of every (document, converter) pair and the Markdown/CSV report.

use crate::kordoc_scorer::KordocScores;
use crate::mdtext;
use crate::reference::{self, Kind};
use crate::score::{self, DocScore};
use crate::{Doc, Run, TOOLS};
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

pub struct Info {
    pub corpus: String,
    pub corpus_total: usize,
    pub seed: u64,
    pub hwp2md_rev: String,
    pub kordoc_version: String,
    pub kordoc_rev: String,
    pub warmup: bool,
}

pub struct DocResult {
    /// `Err` when the reference could not be read (e.g. encrypted HWPX);
    /// such documents are left out of the quality figures.
    pub reference: Result<(), String>,
    pub runs: [Run; 2],
    pub scores: [Option<DocScore>; 2],
    pub md_bytes: [Option<u64>; 2],
}

pub fn score_all(docs: &[Doc], out: &Path, runs: [&Vec<Run>; 2]) -> Vec<DocResult> {
    let score_one = |i: usize| -> DocResult {
        let doc = &docs[i];
        let reference = reference::extract(&doc.path);
        let mut result = DocResult {
            reference: reference.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")),
            runs: [runs[0][i].clone(), runs[1][i].clone()],
            scores: [None, None],
            md_bytes: [None, None],
        };
        for (t, tool) in TOOLS.iter().enumerate() {
            let md_path = out.join(tool).join(format!("{}.md", doc.idx));
            let md = result.runs[t]
                .ok
                .then(|| std::fs::read_to_string(&md_path).ok())
                .flatten();
            result.md_bytes[t] = md.as_ref().map(|m| m.len() as u64);
            if let Ok(r) = &reference {
                let s = match &md {
                    Some(m) => score::score(r, &mdtext::analyze(m)),
                    None => DocScore::failed(r),
                };
                let mut text = String::new();
                for (kind, unit, matched, total) in &s.missed {
                    let _ = writeln!(text, "[{} {matched}/{total}] {unit}", kind.label());
                }
                for run in &s.noise_runs {
                    let _ = writeln!(text, "[잡음 {}자] {run}", run.chars().count());
                }
                // Written even when empty, so a file left by an earlier run
                // in the same output directory never shows stale misses.
                let path = out.join(tool).join(format!("{}.missed.txt", doc.idx));
                if !text.is_empty() || path.exists() {
                    std::fs::write(path, text).ok();
                }
                result.scores[t] = Some(s);
            }
        }
        result
    };

    // Scoring is independent per document; spread it over the cores.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut slots: Vec<Option<DocResult>> = (0..docs.len()).map(|_| None).collect();
    let results = std::sync::Mutex::new(&mut slots);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= docs.len() {
                    break;
                }
                let r = score_one(i);
                results.lock().expect("no poisoned lock")[i] = Some(r);
            });
        }
    });
    slots
        .into_iter()
        .map(|r| r.expect("every document scored"))
        .collect()
}

#[derive(Default)]
struct Agg {
    ok: usize,
    chars: HashMap<Kind, (usize, usize)>,
    md_chars: usize,
    md_covered: usize,
    order_found: usize,
    order_lis: usize,
    tok_ref: usize,
    tok_out: usize,
    tok_recall_hit: usize,
    tok_precision_hit: usize,
    tables_ref: usize,
    tables_out: usize,
    tables_hit: usize,
    headings: usize,
}

impl Agg {
    fn add(&mut self, s: &DocScore) {
        for (k, &(t, m)) in &s.chars {
            let e = self.chars.entry(*k).or_default();
            e.0 += t;
            e.1 += m;
        }
        self.md_chars += s.md_chars;
        self.md_covered += s.md_covered;
        self.order_found += s.order_found;
        self.order_lis += s.order_lis;
        self.tok_ref += s.tok_ref;
        self.tok_out += s.tok_out;
        self.tok_recall_hit += s.tok_recall_hit;
        self.tok_precision_hit += s.tok_precision_hit;
        self.tables_ref += s.tables_ref;
        self.tables_out += s.tables_out;
        self.tables_hit += s.tables_shape_hit;
        self.headings += s.headings;
    }

    fn kind(&self, k: Kind) -> (usize, usize) {
        self.chars.get(&k).copied().unwrap_or((0, 0))
    }

    fn main(&self) -> (usize, usize) {
        Kind::ALL
            .iter()
            .filter(|k| k.is_main())
            .map(|&k| self.kind(k))
            .fold((0, 0), |(a, b), (c, d)| (a + c, b + d))
    }
}

fn pct(num: usize, den: usize) -> String {
    if den == 0 {
        "-".into()
    } else {
        format!("{:.2}%", num as f64 * 100.0 / den as f64)
    }
}

fn ratio(score: &DocScore) -> f64 {
    let (t, m) = score.main_chars();
    if t == 0 {
        1.0
    } else {
        m as f64 / t as f64
    }
}

struct Timing {
    total: f64,
    median: f64,
    p95: f64,
    max: f64,
}

fn timing(mut ms: Vec<f64>) -> Timing {
    if ms.is_empty() {
        return Timing {
            total: 0.0,
            median: 0.0,
            p95: 0.0,
            max: 0.0,
        };
    }
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((q * ms.len() as f64).ceil() as usize).clamp(1, ms.len()) - 1];
    Timing {
        total: ms.iter().sum(),
        median: at(0.5),
        p95: at(0.95),
        max: ms[ms.len() - 1],
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn kind_ratio(s: &DocScore, k: Kind) -> String {
    match s.chars.get(&k) {
        Some(&(t, m)) if t > 0 => format!("{:.4}", m as f64 / t as f64),
        _ => String::new(),
    }
}

fn write_csv(
    out: &Path,
    docs: &[Doc],
    results: &[DocResult],
    kordoc: Option<&KordocScores>,
) -> Result<()> {
    let mut csv = String::from(
        "idx,file,tool,ok,ms,error,md_bytes,ref_main_chars,main_recall,body_recall,cell_recall,\
         textbox_recall,note_recall,header_footer_recall,noise,order,token_recall,\
         token_precision,tables_ref,tables_out,table_shape_hits,headings,\
         k_recall,k_phantom,k_order,k_tables,k_table_exact,k_cell_f1\n",
    );
    for (doc, r) in docs.iter().zip(results) {
        for (t, tool) in TOOLS.iter().enumerate() {
            let run = &r.runs[t];
            let mut row = vec![
                doc.idx.clone(),
                csv_field(&doc.rel),
                tool.to_string(),
                run.ok.to_string(),
                format!("{:.2}", run.ms),
                csv_field(run.error.as_deref().unwrap_or("")),
                r.md_bytes[t].map(|b| b.to_string()).unwrap_or_default(),
            ];
            match &r.scores[t] {
                Some(s) => {
                    let f = |n: usize, d: usize| {
                        if d == 0 {
                            String::new()
                        } else {
                            format!("{:.4}", n as f64 / d as f64)
                        }
                    };
                    row.extend([
                        s.main_chars().0.to_string(),
                        format!("{:.4}", ratio(s)),
                        kind_ratio(s, Kind::Body),
                        kind_ratio(s, Kind::Cell),
                        kind_ratio(s, Kind::TextBox),
                        kind_ratio(s, Kind::Note),
                        kind_ratio(s, Kind::HeaderFooter),
                        f(s.md_chars - s.md_covered, s.md_chars),
                        f(s.order_lis, s.order_found),
                        f(s.tok_recall_hit, s.tok_ref),
                        f(s.tok_precision_hit, s.tok_out),
                        s.tables_ref.to_string(),
                        s.tables_out.to_string(),
                        s.tables_shape_hit.to_string(),
                        s.headings.to_string(),
                    ]);
                }
                None => row.extend(std::iter::repeat_n(String::new(), 15)),
            }
            match kordoc.and_then(|k| k.rows.get(&doc.idx)) {
                Some(k) => {
                    let k = &k[t];
                    let phantom = if k.md_chars == 0 {
                        0.0
                    } else {
                        k.phantom_chars as f64 / k.md_chars as f64
                    };
                    row.extend([
                        format!("{:.4}", k.recall),
                        format!("{phantom:.4}"),
                        format!("{:.4}", k.order),
                        k.tables.to_string(),
                        k.table_exact.to_string(),
                        format!("{:.4}", k.cell_f1),
                    ]);
                }
                None => row.extend(std::iter::repeat_n(String::new(), 6)),
            }
            csv.push_str(&row.join(","));
            csv.push('\n');
        }
    }
    std::fs::write(out.join("results.csv"), csv)?;
    Ok(())
}

fn write_kordoc_section(w: &mut String, info: &Info, k: &KordocScores) -> std::fmt::Result {
    let [h, o] = &k.summary;
    writeln!(w, "## kordoc 채점기 기준\n")?;
    writeln!(
        w,
        "kordoc {} `{}` 의 `bench/compare-md-parsers.mjs` 채점(정답 `bench/ref/hwpx-ref.mjs`, \
         `scoreMd`·`summarize` 는 그 스크립트 소스에서 그대로 가져와 실행)을 두 출력에 똑같이 적용했다. \
         채점 문서 {}개 (정답을 못 만든 문서·kordoc이 ENCRYPTED라 한 문서는 그 스크립트처럼 뺀다).\n",
        info.kordoc_version, info.kordoc_rev, h.docs
    )?;
    writeln!(w, "| 지표 | hwp2md | kordoc |")?;
    writeln!(w, "|---|---:|---:|")?;
    let p = |x: f64| format!("{:.2}%", x * 100.0);
    writeln!(
        w,
        "| **글 재현율** | **{}** | **{}** |",
        p(h.recall),
        p(o.recall)
    )?;
    writeln!(
        w,
        "| └ 변환 성공 문서만 | {} | {} |",
        p(h.recall_parsed),
        p(o.recall_parsed)
    )?;
    writeln!(
        w,
        "| 가짜 글 (낮을수록 좋음) | {} | {} |",
        p(h.phantom),
        p(o.phantom)
    )?;
    writeln!(w, "| 읽기 순서 | {} | {} |", p(h.order), p(o.order))?;
    writeln!(
        w,
        "| 글 누락 0 문서 | {}/{} | {}/{} |",
        h.full_recall_docs, h.docs, o.full_recall_docs, o.docs
    )?;
    writeln!(
        w,
        "| 표 완전 일치 (정답: 보이는 표 {}) | {} ({}) | {} ({}) |",
        h.tables,
        h.table_exact,
        p(h.table_exact_rate),
        o.table_exact,
        p(o.table_exact_rate)
    )?;
    writeln!(w, "| 칸 F1 | {:.4} | {:.4} |", h.cell_f1, o.cell_f1)?;
    writeln!(w, "| 실패 | {} | {} |\n", h.failed, o.failed)?;
    writeln!(
        w,
        "- 글: 정답 문단·칸·글상자 글을 평문에 정렬(`bench/lib/align.mjs`) — 재현율, 참조에 없는 본문 문자 비율(가짜 글), \
         고유 본문 문단(본문 문자 4자 이상)의 최장 증가 부분열 비율(읽기 순서, 문서 평균)."
    )?;
    writeln!(
        w,
        "- 표: 파이프 표·HTML 표를 격자로 만들어 정답과 대조 — 칸 짜임까지 같은 표(완전 일치)와 칸 F1(문서 평균). \
         정답 표는 한글 화면에 보이는 표(`bench/ref/visible-tables.mjs`: 보이는 테두리로 행 띠를 나누고, 칸 분수는 수식으로)이고 \
         1열 표는 뺀다."
    )?;
    writeln!(
        w,
        "- kordoc이 자기 출력 방식과 함께 다듬어 온 채점기라 표현 선택(틀 표 풀기, 칸 글 줄바꿈 등)이 kordoc과 같을수록 \
         유리하다. 정답 표 정의는 위의 이 도구 채점과 같다.\n"
    )?;
    Ok(())
}

/// Write `report.md` and `results.csv`; returns the report text.
pub fn write(
    out: &Path,
    info: &Info,
    docs: &[Doc],
    results: &[DocResult],
    kordoc: Option<&KordocScores>,
) -> Result<String> {
    write_csv(out, docs, results, kordoc)?;

    let n = docs.len();
    let mut agg: [Agg; 2] = Default::default();
    for r in results {
        for (t, a) in agg.iter_mut().enumerate() {
            if r.runs[t].ok {
                a.ok += 1;
            }
            if let Some(s) = &r.scores[t] {
                a.add(s);
            }
        }
    }
    let ref_ok = results.iter().filter(|r| r.reference.is_ok()).count();

    // Speed on the documents both converters handled.
    let both: Vec<usize> = (0..n)
        .filter(|&i| results[i].runs.iter().all(|r| r.ok))
        .collect();
    let both_bytes: u64 = both.iter().map(|&i| docs[i].bytes).sum();
    let times: Vec<Timing> = (0..2)
        .map(|t| timing(both.iter().map(|&i| results[i].runs[t].ms).collect()))
        .collect();

    let mut md = String::new();
    let w = &mut md;
    writeln!(w, "# hwp2md vs kordoc — HWPX → Markdown 비교\n")?;
    writeln!(
        w,
        "- 말뭉치: `{}` — {}개 중 {n}개 표본 (seed {})",
        info.corpus, info.corpus_total, info.seed
    )?;
    writeln!(
        w,
        "- hwp2md `{}` · kordoc {} `{}` (CLI 기본 옵션과 같은 `parse(buf, {{ filePath }})`)",
        info.hwp2md_rev, info.kordoc_version, info.kordoc_rev
    )?;
    writeln!(
        w,
        "- 정답: HWPX XML의 `<hp:t>` 글을 직접 읽음 — 정답을 읽은 문서 {ref_ok}/{n}개로 품질을 잼"
    )?;
    writeln!(
        w,
        "- 시간: 두 도구 모두 프로세스 안에서 문서마다 읽기+변환+쓰기를 잼{}\n",
        if info.warmup {
            " (첫 문서로 한 번 예열)"
        } else {
            ""
        }
    )?;

    writeln!(w, "## 요약\n")?;
    writeln!(w, "| 지표 | hwp2md | kordoc |")?;
    writeln!(w, "|---|---:|---:|")?;
    let mut row = |label: &str, f: &dyn Fn(&Agg) -> String| -> std::fmt::Result {
        writeln!(w, "| {label} | {} | {} |", f(&agg[0]), f(&agg[1]))
    };
    row("변환 성공", &|a| format!("{}/{n}", a.ok))?;
    row("**글 재현율** (본문+표 칸+글상자)", &|a| {
        let (t, m) = a.main();
        format!("**{}**", pct(m, t))
    })?;
    for k in [Kind::Body, Kind::Cell, Kind::TextBox] {
        row(&format!("└ {}", k.label()), &|a| {
            let (t, m) = a.kind(k);
            pct(m, t)
        })?;
    }
    row("각주·미주 글 재현율", &|a| {
        let (t, m) = a.kind(Kind::Note);
        pct(m, t)
    })?;
    row("머리말·꼬리말 글 재현율 (참고)", &|a| {
        let (t, m) = a.kind(Kind::HeaderFooter);
        pct(m, t)
    })?;
    row(
        "누름틀 안내문 재현율 (참고, 빼도 정답)",
        &|a| {
            let (t, m) = a.kind(Kind::Placeholder);
            pct(m, t)
        },
    )?;
    row(
        "잡음 (정답에 없는 글자, 낮을수록 좋음)",
        &|a| pct(a.md_chars - a.md_covered, a.md_chars),
    )?;
    row("읽기 순서", &|a| pct(a.order_lis, a.order_found))?;
    row("어절 재현율 (띄어쓰기까지)", &|a| {
        pct(a.tok_recall_hit, a.tok_ref)
    })?;
    row("어절 정밀도", &|a| pct(a.tok_precision_hit, a.tok_out))?;
    let tables_ref = agg[0].tables_ref;
    row(
        &format!("표 수 (2열 이상, 정답: 보이는 표 {tables_ref})"),
        &|a| a.tables_out.to_string(),
    )?;
    row("표 모양(행×열) 일치", &|a| {
        pct(a.tables_hit, a.tables_ref)
    })?;
    row("제목 줄 수", &|a| a.headings.to_string())?;
    writeln!(
        w,
        "| 변환 시간 합계 (둘 다 성공한 {}개) | {:.0} ms | {:.0} ms |",
        both.len(),
        times[0].total,
        times[1].total
    )?;
    writeln!(
        w,
        "| 문서당 중앙값 / p95 / 최대 | {:.1} / {:.1} / {:.0} ms | {:.1} / {:.1} / {:.0} ms |",
        times[0].median, times[0].p95, times[0].max, times[1].median, times[1].p95, times[1].max
    )?;
    let mbps = |t: &Timing| {
        if t.total > 0.0 {
            format!("{:.1} MB/s", both_bytes as f64 / 1e6 / (t.total / 1000.0))
        } else {
            "-".into()
        }
    };
    writeln!(
        w,
        "| 처리량 | {} | {} |\n",
        mbps(&times[0]),
        mbps(&times[1])
    )?;

    if let Some(k) = kordoc {
        write_kordoc_section(w, info, k)?;
    }

    writeln!(w, "## 지표 설명\n")?;
    writeln!(
        w,
        "- **글 재현율**: 정답 문단의 글자(문자·숫자만, 공백·문장부호·마크업 무시) 가운데 출력에서 찾은 비율. \
         문단째 못 찾으면 12글자 조각으로 나눠 찾는다. 변환에 실패한 문서는 0으로 센다."
    )?;
    writeln!(
        w,
        "- **잡음**: 출력 글자 가운데 어느 정답 문단과도 맞지 않은 비율 (자동 번호·그림 설명·중복 출력 등)."
    )?;
    writeln!(
        w,
        "- **읽기 순서**: 찾은 문단 위치를 정답 순서로 늘어놓았을 때 최장 증가 부분열의 비율."
    )?;
    writeln!(
        w,
        "- **어절**: 공백으로 나눈 낱말을 다중집합으로 비교 — 띄어쓰기가 사라지거나 붙으면 깎인다. \
         정밀도는 출력 어절 가운데 정답(숨은 설명 제외 전체)에 있는 비율."
    )?;
    writeln!(
        w,
        "- **누름틀 안내문**: 채우지 않은 누름틀(`CLICK_HERE`, `dirty≠1`)에 안내 글(`Direction`)이 그대로 든 것. \
         한컴이 인쇄하지 않아 본문 재현율에서 뺐다. 출력해도 잡음으로 치지 않는다. 안내 글과 다른 값(`0000.00.00.` 같은 \
         미리 넣은 값)은 보통 글이다."
    )?;
    writeln!(
        w,
        "- **표**: 정답 표는 한글 화면에 보이는 표다. `hp:tbl` 하나를 칸 테두리(`header.xml` `borderFill`)로 행 띠에 나눠, \
         보이는 선이 있는 행 묶음마다 표 하나로 치고 나머지 행은 글(본문)로 친다. 선 없는 빈 여백 행·빈 끝 열·안 쓰는 격자선은 \
         접고, 선 하나로 위아래가 나뉜 짧은 두 칸은 분수로 합친다(kordoc `bench/ref/visible-tables.mjs` 와 같은 정의, 따로 구현). \
         2열 이상 표만 센다 (1열 표는 꾸밈 틀이 많아 문단으로 내는 것도 정답). 모양은 그 행×열과 출력 표(파이프 표·HTML 표, \
         colspan/rowspan 격자 반영)의 행×열이 같은 수.\n"
    )?;

    // Documents with the largest recall gap.
    let mut gaps: Vec<(usize, f64, f64)> = (0..n)
        .filter_map(|i| {
            let [Some(a), Some(b)] = &results[i].scores else {
                return None;
            };
            Some((i, ratio(a), ratio(b)))
        })
        .collect();
    gaps.sort_by(|x, y| (y.1 - y.2).abs().total_cmp(&(x.1 - x.2).abs()));
    writeln!(w, "## 글 재현율 차이가 큰 문서 (상위 10)\n")?;
    writeln!(w, "| # | 문서 | hwp2md | kordoc |")?;
    writeln!(w, "|---|---|---:|---:|")?;
    for &(i, a, b) in gaps.iter().take(10) {
        writeln!(
            w,
            "| {} | {} | {:.1}% | {:.1}% |",
            docs[i].idx,
            docs[i].rel,
            a * 100.0,
            b * 100.0
        )?;
    }
    writeln!(w)?;

    for (t, tool) in TOOLS.iter().enumerate() {
        let mut worst: Vec<(usize, f64)> = (0..n)
            .filter_map(|i| results[i].scores[t].as_ref().map(|s| (i, ratio(s))))
            .collect();
        worst.sort_by(|x, y| x.1.total_cmp(&y.1));
        writeln!(w, "## {tool} 글 재현율이 가장 낮은 문서 (5)\n")?;
        writeln!(w, "| # | 문서 | 재현율 |")?;
        writeln!(w, "|---|---|---:|")?;
        for &(i, v) in worst.iter().take(5) {
            writeln!(
                w,
                "| {} | {} | {:.1}% |",
                docs[i].idx,
                docs[i].rel,
                v * 100.0
            )?;
        }
        writeln!(w)?;
    }

    let failures: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (0..2).map(move |t| (i, t)))
        .filter(|&(i, t)| !results[i].runs[t].ok)
        .collect();
    if !failures.is_empty() {
        writeln!(w, "## 변환 실패\n")?;
        writeln!(w, "| 도구 | # | 문서 | 오류 |")?;
        writeln!(w, "|---|---|---|---|")?;
        for (i, t) in failures {
            let err = results[i].runs[t].error.as_deref().unwrap_or("");
            writeln!(
                w,
                "| {} | {} | {} | {} |",
                TOOLS[t],
                docs[i].idx,
                docs[i].rel,
                err.replace('|', "\\|")
            )?;
        }
        writeln!(w)?;
    }
    let ref_fail: Vec<usize> = (0..n).filter(|&i| results[i].reference.is_err()).collect();
    if !ref_fail.is_empty() {
        writeln!(w, "## 정답을 읽지 못한 문서 (품질 지표에서 제외)\n")?;
        for i in ref_fail {
            let err = results[i]
                .reference
                .as_ref()
                .err()
                .map_or("", String::as_str);
            writeln!(w, "- {} `{}` — {}", docs[i].idx, docs[i].rel, err)?;
        }
        writeln!(w)?;
    }
    writeln!(
        w,
        "출력: `hwp2md/NNNN.md`, `kordoc/NNNN.md` (번호는 `files.tsv`), 못 찾은 정답 문단과 긴 잡음은 \
         `<도구>/NNNN.missed.txt`, 문서별 수치는 `results.csv`."
    )?;

    std::fs::write(out.join("report.md"), &md)?;
    Ok(md)
}
