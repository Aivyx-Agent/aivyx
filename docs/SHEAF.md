# Structured-Data Readers — CSV / XLSX / PDF (Chapter Sheaf)

> **Status:** 🟡 **PLANNED (SH.0–SH.5).** The third **new-tools breadth** chapter,
> the next item off the [Atlas §6 backlog](ATLAS.md) after [Forge](FORGE.md)
> (`web.extract` + `git.commit`) and [Abacus](ABACUS.md) (the utilities pack): a
> small set of **structured-data reader tools** that turn an operator file the
> agent can already reach into *legible structured content* — `data.csv`,
> `data.xlsx`, and `data.pdf`. They are the file-content analogue of `web.extract`:
> `fs.read` hands the LLM raw bytes and it burns context (and often fails) parsing
> them; a reader returns rows / cells / text it can actually use. The keystone
> decision: each reader **reuses the existing `fs.read` capability and the
> `aivyx-core` filesystem sandbox** (the `lexical_resolve` escape-guard Chapter Z
> made `pub`), so it adds **no new capability base** and **no new I/O reach** — it
> only *parses bytes the agent is already permitted to read*. That makes it an
> **infrastructure-tier** addition (no P10 substrate amendment). New dependencies
> are isolated in a focused crate; opt-in by registration, default off.

## 1. Why this chapter

`fs.read` returns a file's raw bytes. For a `.csv` the LLM then re-derives the
column structure token-by-token; for an `.xlsx` it can't (binary zip); for a
`.pdf` it gets a wall of binary. The agent routinely needs the *content* of these
formats — an expenses CSV, a spreadsheet of inventory, a PDF invoice — and today
the only honest answer is "I can't read that." Three readers close the gap:

- **`data.csv`** — a delimited-text file → `{headers, rows, row_count}` (capped).
- **`data.xlsx`** — a spreadsheet → a named sheet's `{headers, rows}` (the binary
  format `fs.read` cannot expose at all).
- **`data.pdf`** — a PDF → extracted `{text, pages}` (the readability pass for
  documents, mirroring what `web.extract` does for web pages).

This is the file-content sibling of `web.extract`: same "raw → legible" shape, one
tier down (it adds no new outbound reach, see §2).

## 2. Architecture & governance decisions (locked)

### Reuse `fs.read` + the fs sandbox — **no new base, no new reach**
Each reader takes a `path`, resolves it through **the exact `FsReadTool` machinery**
— `lexical_resolve(sandbox_root, path)` to derive an `fs.read:<resolved>` scope,
then the canonical re-resolve + `starts_with(fs_root)` fence at execute time (the
"defense in depth" the fs tools document). So a reader can only ever read a file
*the agent could already `fs.read`*; it inherits the access-level `fs_root`, the
escape-guard, and the size caps for free. **No new `KNOWN_BASES` entry** — the
capability *is* `fs.read` (parsing is not a new capability), exactly as
`web.extract` reused `net.fetch`.

### Infrastructure tier — **not** substrate, **no P10 amendment**
This is the decisive governance line vs. Forge. `web.extract` was **substrate**
(it performs genuinely new *outbound I/O* — an HTTP GET — so it grew the P10 count
13→15). A structured-data reader performs **no new I/O**: it is a pure *transform
over bytes the agent already obtained via `fs.read`*. A transform that adds no
reach is the **infrastructure** character — the same class as `tools.list` (a view
over the registry) or `graph.read` (a view over derived memory), neither of which
touched the P10 substrate cap. So Sheaf grows the **uncapped infrastructure tier**
and needs **no PRODUCT.md P10 substrate-count amendment**. *(Rejected alternative:
treat them as substrate beside `web.extract`/`fs` and pay a +3 P10 amendment — but
"parses already-readable bytes" is not a new irreducible capability, so substrate
overstates it.)*

### Heavy parsing deps isolated in a focused crate
`data.xlsx` and `data.pdf` need real parsers (a zip/XML reader; a PDF text
extractor). Pulling those into `aivyx-core` would bloat the agent's hottest crate.
So the readers live in a **new focused crate, `aivyx-dataread`**, that depends on
`aivyx-core` for the `Tool` trait + the `pub lexical_resolve` sandbox helper, and
owns the format deps. The agent assembly registers the readers the same way it
registers any infrastructure tool. *(Open question OQ-2 weighs putting `data.csv`
— whose dep is already in the tree — directly in `aivyx-core` and only spinning the
crate up for xlsx/pdf; lean to one crate for all three for cohesion.)*

### Read-only, capped, license-clean
- **Read-only.** No reader writes, converts, or mutates a file. `data.*` is a
  pure projection; there is no `data.write`.
- **Capped.** Each reader caps rows / cells / extracted-text length (mirroring
  `fs.read`'s 256 KiB truncation) so a huge file can't blow the context or memory.
  Truncation is flagged in the output (`truncated: true`).
- **Permissive deps only.** `csv` (already in the tree — MIT/Unlicense), `calamine`
  for xlsx (MIT), and a PDF text extractor — `pdf-extract` / `lopdf` (MIT). All
  must clear `deny.toml` (BUSL project, permissive-only); verified per-phase via
  `cargo deny`. **PDF is the licensing/robustness risk** and is phased last so it
  can be deferred without holding up CSV/XLSX if no extractor clears cleanly.

## 3. Scope

**In:** the three readers (`data.csv`, `data.xlsx`, `data.pdf`); the `aivyx-dataread`
crate; the `fs.read` + sandbox reuse; row/cell/text caps with a `truncated` flag;
their tests; `docs/TOOLS.md` catalog entries + the `check_tool_quality` sweep over
the new crate; a live-verify. **Out:** writing/converting files (`data.*` is
read-only); a new capability base or P10 amendment; OCR or image extraction from
PDFs (text layer only); streaming/iterator access to gigantic files (cap + truncate
instead); the rest of the §6 backlog (keyed integrations — GitHub/weather/Google
Tasks — each a Chapter-F tool-process, their own chapter).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **SH.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **SH.1** ✅ | **`data.csv` + the sandbox-reuse spine** | DONE. New `aivyx-dataread` crate (workspace + default-members). `ReaderSandbox` carries the shared spine — derives the `fs.read:<resolved>` scope and runs both fences (`lexical_resolve` then canonicalize + `starts_with` root) exactly like `FsReadTool`, capping raw bytes at 8 MiB. `data.csv` (`csv` crate, already in tree) → `{path, headers, rows, row_count, truncated}` with delimiter/`\t`, `has_headers`, `max_rows` (default 1000, hard cap 100k) options + per-cell 4096-char cap; ragged rows tolerated, quoting/embedded-newlines handled. **Registered in the daemon** (`aivyx-cli`) beside `fs.read` from the same canonical root — no new scope (the held `fs.read:<root>/**` covers it), ceiling-stripped for SemiTrusted like `fs.read`. `docs/TOOLS.md` entry added. 14 crate tests + binary builds; full workspace suite + clippy `-D warnings` green; **no tool-count assertion broke**. |
| **SH.2** ✅ | **`data.xlsx`** | DONE. `calamine` 0.26 (MIT — `cargo deny` licenses/advisories clean) parses the binary zip+XML `fs.read` can't expose, in-memory from the sandbox-read bytes (no temp files). Reuses the `ReaderSandbox` spine (same `fs.read` gate). `{path, sheet, sheet_names, headers, rows, row_count, truncated}`; selects a named `sheet` (errors with the available list) or the first; cells stringified (numbers/dates/bools/text, empty→`""`), shared `cap_cell` + `max_rows` caps; a truncated-read corruption is reported with a hint. The `cap_cell`/`MAX_CELL_CHARS` helper was lifted into `sandbox.rs` for both readers. **Registered in the daemon** beside `data.csv` (shared sandbox). `docs/TOOLS.md` entry added. 6 new tests (row transform via a programmatically-built `Range` — no binary fixture; headers/no-headers/truncate/empty-cell/bad-bytes/metadata); 20 crate tests; binary builds; clippy `-D warnings` clean. |
| **SH.3** | **`data.pdf`** | Add a PDF text extractor (`pdf-extract`/`lopdf`, MIT — license-gated; **deferrable** if none clears) → `{text, pages, truncated}`. Text layer only, no OCR. |
| **SH.4** | **Catalog + quality sweep** | `docs/TOOLS.md` entries for the three readers (scope `fs.read`, infrastructure); a `check_tool_quality` sweep over `aivyx-dataread` (extends the Atlas AT.3 guard, as Abacus did for the toolkit tier). |
| **SH.5** | **Finalize** | Live-verify each reader against a real sample file through a real `ToolContext` (these are in-process infra tools, so the verify is an integration test that constructs the tool with a temp `fs_root` + sample file and asserts the parsed output — not the toolkit-IPC trick Abacus used); full workspace suite + clippy + `cargo deny` green; `docs/ATLAS.md` §6 updated (readers ✅); chapter memory recorded; status → COMPLETE. |

**Discipline:** SH.1 carries the sandbox-reuse + crate spine so SH.2/SH.3 are pure
parser adds. SH.3 (PDF) is isolated last and explicitly deferrable on a licensing
miss. Test band: **moderate** — parser-heavy but file-fixture-driven; price
**~25–35 new tests**, heaviest in SH.1 (the sandbox fence + CSV edge cases:
quoting, embedded newlines, ragged rows) and SH.2 (sheet selection + cell typing).

## 5. Open questions (resolve in-phase)

- **OQ-1 — tier (SH.1).** Infrastructure (locked — a transform over `fs.read`-able
  bytes, no new reach, no P10 amendment) vs. substrate beside `web.extract` (a +3
  P10 amendment). Locked to infrastructure; flagged for review because it is the
  load-bearing governance call.
- **OQ-2 — crate home (SH.1).** One `aivyx-dataread` crate for all three (locked
  default, cohesion) vs. `data.csv` in `aivyx-core` (its dep is already present)
  with a crate only for xlsx/pdf. Lean one crate.
- **OQ-3 — PDF extractor + license (SH.3).** `pdf-extract` vs. `lopdf` vs. `pdf`;
  resolve against `cargo deny` + extraction quality on a real PDF. **Defer
  `data.pdf`** if no permissive option extracts usable text cleanly — CSV + XLSX
  still ship the chapter.
- **OQ-4 — output caps + shape (SH.1/SH.2).** Row cap, cell-length cap, extracted-
  text cap (mirror `fs.read`'s 256 KiB); typed cells vs. all-strings for xlsx;
  which sheet `data.xlsx` defaults to (first vs. required `sheet` arg).
- **OQ-5 — CSV dialect (SH.1).** Delimiter auto-detect vs. an explicit `delimiter`
  arg (covering TSV); header row present vs. inferred. Keep it explicit and small.

---

*Chapter Sheaf finishes the "make files legible" thought `web.extract` started for
the web: the agent can already reach an operator's files via `fs.read`, but a CSV,
a spreadsheet, or a PDF arrives as bytes it can't use. Three readers — same sandbox,
same `fs.read` capability, one tier down because they add no new reach — hand the
LLM rows, cells, and text instead. No new capability, no new reach, no P10
amendment: just the parsers, isolated in their own crate, turning files the agent
may already read into content it can actually understand.*
