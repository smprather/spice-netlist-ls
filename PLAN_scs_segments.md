# Plan: per-section dialect switching for `.scs` files (`simulator lang=…`)

> **Status (2026-08-26): Implemented and shipped in `v0.3.0` (`7aacd59`) – `src/segments.rs`, `src/formatter.rs:30`, `src/linter.rs:233`, `src/bin/ls.rs:246` all route per-section; `tests/scs_segments.rs` + `testdata/scs/` fixtures are frozen. Current `main` is `v2026.8.0` (calver) with 14 formatter rules (`src/formatter.rs:7`) and statement-level `fmt: off/on/skip` pragmas. This doc retained as design record.

## Goal

A `.scs` (Spectre netlist) file is a sequence of *language segments*. A
`simulator lang=spice` line means "parse the following lines as SPICE until
the next `simulator lang=spectre`"; `simulator lang=spectre` switches back.
Today the whole file is parsed under the single dialect chosen by
`detect_dialect`, so a mixed deck is silently mis-formatted / mis-linted at
segment boundaries (wrong `key=value` spacing, wrong inline-comment delim
(`;` vs `//` vs `$`), wrong comment-line recognition). This plan makes the
active dialect **per-section**, so mixed `.scs` decks become first-class.

Non-goal: implementing Spectre's *native* netlist language beyond what the
existing `spectre` dialect already covers. Segmentation only routes each
section to the right existing dialect; it adds no new grammar.

## Design invariant (must hold throughout)

- **No `simulator lang=` lines ⇒ behave exactly as today** (whole file under
  the detected/`--dialect` dialect). Segmentation is opt-in per file: it
  activates only when a `simulator lang=` directive is present. This keeps
  every existing test and every plain `.sp`/`.lib`/`.net` deck unchanged.
- **`simulator lang=…` lines are structural, not comments**, and pass through
  the formatter **verbatim** (they are not normalized, lowercased, or
  re-spaced — they are Spectre syntax the simulator reads literally).
- **Output is byte-identical to today for any file without `simulator lang=`.**
  This is the primary regression guard.

## Step 0 — fixtures + golden baseline (do this first, before any code)

1. Add test fixtures under `testdata/scs/`:
   - `lang_spice_only.scs` — opens with `simulator lang=spice`, SPICE body,
     no switch back. (The case that already "works" today; becomes the
     correctness anchor.)
   - `lang_mixed.scs` — `lang=spice` segment, then `lang=spectre` segment,
     then back to `lang=spice`. Exercises a SPICE `; comment` in the spice
     section (which the `spectre` dialect would mis-handle) and a `//`
     comment + paren-node instance in the spectre section.
   - `lang_spectre_only.scs` — native spectre, no `simulator lang` at all
     (unchanged-behavior guard).
   - `no_lang.sp` — plain SPICE, unchanged-behavior guard.
2. Snapshot the **current** `spicefmt`/`spicefmt --lint` output for each
   (`cargo run --release -- > …`) into `testdata/scs/golden_before/`. These
   are the "must-not-change" references for `no_lang.sp`,
   `lang_spectre_only.scs`, and (modulo the bug being fixed) the spice
   sections of the mixed file. **Commit these.** They let a clean context
   prove it didn't regress plain files.
3. Add an insta snapshot test (`tests/scs_segments.rs`) that formats +
   lints each fixture and snapshots the result. The
   `no_lang.sp`/`lang_spectre_only.scs` snapshots are frozen from step 2 and
   must not change; the `lang_*` snapshots are the *new* expected output
   (updated once the fix lands, then frozen).

## Step 1 — the segmentation pre-pass (`src/segments.rs`, new module)

A pure function that splits the input into sections by `simulator lang=`
directives. This is the only new piece of logic; everything else is wiring.

```rust
/// A run of physical lines parsed under one dialect, plus the directive
/// line that started it (if any — the file's first section has none).
pub struct Section<'a> {
    /// The `simulator lang=…` line verbatim, or `None` for the implicit
    /// pre-switch section. Emitted verbatim by the formatter.
    pub header: Option<&'a str>,
    pub dialect: DialectKind,
    /// Physical line indices [start, end) of this section's *body*
    /// (excluding the header line).
    pub line_range: (usize, usize),
}
```

```rust
pub fn segments(input: &str, fallback: DialectKind) -> Vec<Section<'_>>
```

Semantics (all case-insensitive, ASCII; the value after `=` is trimmed):
- Scan physical lines for ones matching `^\s*simulator\s+lang\s*=\s*(spice|spectre)\b`.
  (`spice` is the only switch Spectre documents in practice; an unknown
  value leaves the section's dialect unchanged and the line still passes
  through verbatim.)
- The implicit section before the first switch uses `fallback` (the result
  of `detect_dialect` for the file). For a file that *starts* with
  `simulator lang=spice`, the implicit pre-section is empty (header line
  is the first real line) and is dropped.
- A `simulator lang=…` line is the **header** of a new section; it is
  excluded from the body's `line_range`. Comment/blank lines between a
  header and the next header belong to the current section's body.
- If the file has **no** `simulator lang=…` line anywhere → return a single
  section covering the whole file with `fallback` and `header: None`. This
  is the fast path; callers detect "one section, no header" and take today's
  code path verbatim (guaranteeing the no-regression invariant).
- Continuation (`+`) lines never span a `simulator lang=` line in practice
  (the directive is a standalone line); the segmenter operates on physical
  lines and does **not** merge continuations. `logical_line_spans` runs
  *within* each section's body, so a `+` at the top of a section can't reach
  into the previous section's last statement — which is correct, because a
  `simulator lang=` line severs continuation attachment (analogous to how a
  comment severs it today).

Tests (`src/segments.rs` `#[cfg(test)]`): single spice section, mixed,
spectre-only (no header), unknown lang value, leading/trailing blanks,
section boundary immediately followed by a `+` (must orphan, not attach).

## Step 2 — thread sections through the three entry points

The design keeps `format_str` / `lint_str` signatures **unchanged** (they
stay `(input, opts|dialect)`); segmentation is internal. The public
`FormatOptions.dialect` becomes the *fallback* dialect when the file has no
`simulator lang=`; when it does, per-section dialect wins.

### 2a — `format_str` / `format_into` (`src/formatter.rs`)

```
format_str(input, opts):
  fallback = opts.dialect
  secs = segments(input, fallback)
  if secs is the single whole-file no-header section:
      # today's path, unchanged
      parse_str + format_into with fallback
      return
  out = String::with_capacity(...)
  for sec in secs:
      if let Some(h) = sec.header: out.push_str(h); out.push('\n')
      body_text = input lines in sec.line_range joined
      sub_opts = opts with dialect = sec.dialect
      dialect = get_dialect(sec.dialect)
      file = parse_str(&body_text, dialect)
      format_into(&file, &mut out, &sub_opts, dialect.as_ref())
  # final-newline / trailing-ws handled once, at the end, as today
  return out
```

Notes:
- `format_into`'s end-of-pass `trim_trailing_whitespace` /
  `insert_final_newline` must run **once over the whole `out`**, not per
  section (so we don't insert/drop a newline at each segment boundary).
  Refactor `format_into` so the per-section loop calls only the statement
  emitter, and the trailer fixups run after the loop. Today's single-section
  path already does this; just don't call the trailer inside the loop.
- `wrap_line` width: `opts.max_width` is global; spectre's
  `continuation_indent` (`+ `) is per-dialect and already read off the
  dialect passed to `format_into` — so passing the section's dialect makes
  wrapping use the right indent automatically. No change needed there.
- The `simulator lang=…` header line is pushed verbatim and is **not** run
  through `wrap_line` (it's short and structural).

### 2b — `lint_str` (`src/linter.rs`)

`lint_str` currently loops over `logical_line_spans(input, dialect)` under
one dialect. Section-aware version:

```
lint_str(input, dialect, opts):
  fallback = dialect.kind()
  secs = segments(input, fallback)
  if single whole-file no-header section:
      # today's path, unchanged
      return lint_str_single(input, dialect, opts)
  diags = Vec::new()
  for sec in secs:
      body = input lines in sec.line_range
      sub_dialect = get_dialect(sec.dialect)
      # line numbers must be global, so offset diagnostics by sec.line_range.0
      for d in lint_str_single(&body, sub_dialect, opts_for_sec):
          d.range.start_line += sec.line_range.0 as u32
          d.range.end_line   += sec.line_range.0 as u32
          diags.push(d)
  diags.sort_by_key + dedup (as today)
  return diags
```

- **Line-number offset is the one easy-to-get-wrong detail.** The LSP and
  CLI both report 0-based lines; `sec.line_range.0` is a physical-line
  offset added to every diagnostic the sub-lint produces. Test this
  explicitly: a floating node in the *second* section must report the
  global line, not the within-section line.
- `external_subckts` (the include walk) is keyed on the file path and
  currently uses one dialect. For sectioned files, run the include walk
  **per section's dialect** and union the results (subckts defined in a
  spice section are visible to a spectre section and vice versa — they
  share a namespace within the file). Simplest: union all per-section
  `external_subckts` maps. This keeps arity/undefined checks correct across
  boundaries.
- Control-flow: `control_depth` (ngspice `.control` blocks) does not apply
  to spectre sections; but ngspice-only constructs won't appear in a
  spectre section anyway, so per-section linting naturally isolates them.

### 2c — LSP (`src/bin/ls.rs`)

Three handlers, each currently builds one dialect via
`detect_dialect(text)`:

- `publish_diagnostics`: build `secs = segments(text, detect_dialect(text))`
  and union diagnostics (offsetting lines as in 2b). One
  `publishDiagnostics` notification per didOpen/didChange, as today — the
  client sees a single merged diagnostic set.
- `textDocument/formatting`: call the (now section-aware) `format_str` —
  no change to the handler beyond what `format_str` already does.
- `textDocument/definition`: `subckt_ref_at_line` currently parses the
  whole file under one dialect. Make it section-aware: find which section
  `position.line` falls in, parse *that section* under its dialect to get
  the ref name, then `find_subckt_def` searches the whole file (all
  sections) + includes for the def. The def-search is dialect-agnostic
  (it just looks for `.subckt NAME`), so it already works across sections
  once the *ref extraction* uses the right dialect.

## Step 3 — detection stays, fallback is per-file

- `detect_dialect` is unchanged: a file with `simulator lang=spice` still
  scores `spectre` (the `simulator lang` marker is decisive-spectre). That
  detected kind becomes the **fallback** for the implicit pre-switch
  section. For a file that *starts* with `simulator lang=spice`, the
  implicit section is empty and the fallback is never used, so the
  detection result is effectively cosmetic for those files — which is fine.
- `--dialect`/`spicefmt.toml` override the fallback only; explicit
  `simulator lang=` lines in the file always win per-section. Document
  this in the README under Dialects.

## Step 4 — tests & verification (the acceptance bar)

1. `cargo test` — all existing tests pass unchanged (the no-regression
   guard). The new `tests/scs_segments.rs` insta snapshots pass.
2. Byte-identical check on the **plain** fixtures:
   `no_lang.sp`, `lang_spectre_only.scs`, and the existing
   `testdata/*.sp`/`.net`/`.lib` files — `spicefmt` output vs
   `golden_before/` must match. (Run the same cross-check the perf work
   used: build before/after binaries, `cmp` outputs.)
3. Behavioral checks on the mixed fixture:
   - a `; comment` in a `lang=spice` section is preserved as a comment
     (under the spectre dialect today it would be mis-tokenized);
   - a `// comment` in a `lang=spectre` section is preserved as a comment;
   - `key = value` spacing in spice sections, `key=value` in spectre
     sections;
   - a floating node in the second section reports the **global** line
     number (the offset bug guard).
4. Idempotency: `spicefmt | spicefmt` is a fixed point on every fixture
   (formatter invariant), including the mixed file.
5. Re-run the perf probe on `examples/auto_clock_0.sp.bz2` (no `simulator
   lang=` → fast path) to confirm the segmentation fast-path adds no
   measurable overhead to plain decks.

## Step 5 — docs

- README "Dialects" section: add a paragraph on `.scs` segmentation — a
  `simulator lang=spice`/`lang=spectre` line switches the active dialect
  for the lines that follow; `--dialect` sets only the fallback for the
  implicit pre-switch section; plain files are unaffected.
- `spicefmt.1` (via `build.rs`): one line in the dialect/notes section.

## Risk register (what to watch)

- **Line-number offset in lint/LSP** (step 2b/2c). Highest risk; mitigated
  by the dedicated offset test in step 4.3.
- **Continuation across a section header.** A `+` immediately after a
  `simulator lang=` line must orphan, not attach to the previous section.
  Covered by the segmenter test in step 1.
- **`external_subckts` union** across dialects (step 2b) — must not double
  count or miss defs. Covered by an arity test: an X in a spectre section
  calling a subckt defined in a spice section of the same file resolves.
- **Trailing-newline/trailing-ws trailer** running once, not per section
  (step 2a). Mitigated by the byte-identical guard on plain files (which
  exercises the trailer) plus the idempotency check on the mixed file.

## Execution order (dependency-respecting)

1. Step 0 (fixtures + golden_before + commit).
2. Step 1 (`segments.rs` + unit tests, no wiring) + commit.
3. Step 2a (format) + commit — verify against `golden_before` for plain
   files and update the mixed-fixture snapshot.
4. Step 2b (lint, with line offset) + commit.
5. Step 2c (LSP) + commit.
6. Step 5 (docs) + commit.
7. Final: step 4 acceptance bar, then version bump + tag + release.

Each step is independently testable and committable; a clean context can
resume at any step by reading the commit history.