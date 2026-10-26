// kordoc conversion driver, started by kordoc-bench.
//
// Converts every listed document in one Node process with the same options
// the kordoc CLI uses (`parse(buffer, { filePath })` → `result.markdown`), so
// the per-document time excludes Node start-up.  Timing covers reading the
// file, parsing and writing the Markdown, like the hwp2md side.
//
// argv: <kordoc dist/index.js> <list.tsv: idx\tabs-path> <out dir> <warmup 0|1>
// stdout: one JSON object per document: { idx, ok, ms, error? }

import { readFile, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { performance } from "node:perf_hooks"

const [indexJs, listFile, outDir, warmup] = process.argv.slice(2)
const { parse } = await import(pathToFileURL(indexJs).href)

const jobs = (await readFile(listFile, "utf8"))
  .split("\n")
  .filter(Boolean)
  .map(line => {
    const [idx, path] = line.split("\t")
    return { idx, path }
  })

const toArrayBuffer = buf => buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength)

async function convert(path) {
  const buf = await readFile(path)
  return parse(toArrayBuffer(buf), { filePath: path })
}

if (warmup === "1" && jobs.length > 0) {
  try { await convert(jobs[0].path) } catch { /* reported in the timed run */ }
}

for (const { idx, path } of jobs) {
  const t0 = performance.now()
  let rec
  try {
    const result = await convert(path)
    if (result.success) {
      await writeFile(join(outDir, `${idx}.md`), result.markdown)
      rec = { idx, ok: true, ms: performance.now() - t0 }
    } else {
      const error = [result.code, result.error].filter(Boolean).join(" ")
      rec = { idx, ok: false, ms: performance.now() - t0, error }
    }
  } catch (err) {
    const error = String(err?.stack ?? err).split("\n")[0]
    rec = { idx, ok: false, ms: performance.now() - t0, error }
  }
  process.stdout.write(JSON.stringify(rec) + "\n")
}
