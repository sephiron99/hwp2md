// Score both converters with kordoc's own Markdown scorer, started by kordoc-bench.
//
// kordoc ships `bench/compare-md-parsers.mjs`, which scores another parser's
// Markdown against kordoc's independent HWPX reference (bench/ref/hwpx-ref.mjs)
// with the same scorer it uses for itself.  That script walks kordoc's private
// bench/corpus, so this driver reuses its parts instead: the reference and
// scoring modules are imported as-is, and `scoreMd` / `summarize` are taken
// verbatim from the script's source at run time — the criteria always match
// the kordoc checkout in use.  Skips mirror the script: documents whose
// reference cannot be built, and documents kordoc reports as ENCRYPTED.
//
// argv: <kordoc dir> <jobs.tsv: idx\thwpx\thwp2md md|-\tkordoc md|-\tkordoc ENCRYPTED 0|1>
// stdout: one JSON object { summary: { hwp2md, kordoc }, rows: [{ idx, hwp2md, kordoc }] }

import { readFile } from "node:fs/promises"
import { join } from "node:path"
import { pathToFileURL } from "node:url"

const [kordocDir, jobsFile] = process.argv.slice(2)
const bench = join(kordocDir, "bench")
const load = rel => import(pathToFileURL(join(bench, rel)).href)

const { extractRef } = await load("ref/hwpx-ref.mjs")
const { normKey, mdToPlain } = await load("lib/normalize.mjs")
const { mdTables } = await load("lib/md-tables.mjs")
const { alignUnits, lisLength } = await load("lib/align.mjs")
const { collectIrGrids, scoreTables } = await load("lib/table-score.mjs")

/** Source text of `function <name>(…) { … }` in `src`, by brace matching. */
function functionSource(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`compare-md-parsers.mjs has no function ${name}`)
  let depth = 0
  for (let i = src.indexOf("{", start); i < src.length; i++) {
    if (src[i] === "{") depth++
    else if (src[i] === "}" && --depth === 0) return src.slice(start, i + 1)
  }
  throw new Error(`unbalanced braces in function ${name}`)
}

const script = await readFile(join(bench, "compare-md-parsers.mjs"), "utf8")
const build = name => new Function(
  "normKey", "mdToPlain", "mdTables", "alignUnits", "lisLength", "collectIrGrids", "scoreTables", "includeSingleCol",
  `return (${functionSource(script, name)})`,
)(normKey, mdToPlain, mdTables, alignUnits, lisLength, collectIrGrids, scoreTables, false)
const scoreMd = build("scoreMd")
const summarize = build("summarize")

const jobs = (await readFile(jobsFile, "utf8")).split("\n").filter(Boolean).map(line => {
  const [idx, hwpx, hwp2mdMd, kordocMd, encrypted] = line.split("\t")
  return { idx, hwpx, hwp2mdMd, kordocMd, encrypted: encrypted === "1" }
})

const readMd = async p => (p === "-" ? null : await readFile(p, "utf8").catch(() => null))

const rows = []
for (const job of jobs) {
  if (job.encrypted) continue
  let ref
  try { ref = await extractRef(await readFile(job.hwpx)) } catch { continue }
  const row = { idx: job.idx }
  for (const [tool, path] of [["hwp2md", job.hwp2mdMd], ["kordoc", job.kordocMd]]) {
    const md = await readMd(path)
    row[tool] = { failed: md === null, ...scoreMd(md ?? "", ref) }
  }
  rows.push(row)
}

process.stdout.write(JSON.stringify({
  summary: { hwp2md: summarize(rows, "hwp2md"), kordoc: summarize(rows, "kordoc") },
  rows,
}) + "\n")
