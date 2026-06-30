<p align="center">
  <img src="docs/vgi-logo.png" alt="Query.Farm VGI" width="180">
</p>

# vgi-pprof

A [VGI](https://query.farm) worker that decodes **pprof** profiles — the gzip-wrapped
`profile.proto` produced by Go, `gperftools`, [Parca](https://www.parca.dev/),
[Pyroscope](https://pyroscope.io/), [py-spy](https://github.com/benfred/py-spy), etc. — into
rows, so SRE and performance teams can **bulk-diff profiles in SQL** and wire CI
performance-regression gates.

`go tool pprof` is interactive and single-file; there is no way to load 500 profiles and ask
*"which function regressed across this deploy?"*. **This is that.** It complements Query Farm's
Pyroscope extension (which pushes the inverse direction) and feeds
[`vgi-symbols`](https://query.farm) the `build_id` of unsymbolized native frames.

```sql
INSTALL vgi FROM community; LOAD vgi;
ATTACH 'pprof' (TYPE vgi, LOCATION '/path/to/pprof-worker');
SET search_path = 'pprof.main';

-- top self-time functions across a directory of CPU profiles
-- (value[2] is cpu/nanoseconds; value[1] is samples/count — see pprof.meta)
SELECT s.frame[1].function AS fn, sum(s.value[2]) AS cpu_ns
FROM glob('/profiles/*.pb.gz') f, pprof.stacks(f.path) s
WHERE s.error IS NULL
GROUP BY 1 ORDER BY 2 DESC LIMIT 20;

-- regression diff: candidate build vs baseline
WITH base AS (
  SELECT frame[1].function AS fn, sum(value[2]) AS ns
  FROM pprof.stacks('baseline.pb.gz') WHERE error IS NULL GROUP BY 1),
cand AS (
  SELECT frame[1].function AS fn, sum(value[2]) AS ns
  FROM pprof.stacks('candidate.pb.gz') WHERE error IS NULL GROUP BY 1)
SELECT cand.fn, cand.ns - COALESCE(base.ns, 0) AS delta
FROM cand LEFT JOIN base USING (fn) ORDER BY delta DESC;
```

## How pprof maps onto SQL

A pprof file decodes once into the proto's string-table-indexed graph; the worker flattens it
into the views below. The `src` argument **overloads** on a `VARCHAR` path (which may be a glob),
a `LIST(VARCHAR)` of paths, or a `BLOB` of inline profile bytes.

| Area | SQL surface |
| --- | --- |
| **Flattened stacks** (headline) | `pprof.stacks(src)` → `sample_id`, `value LIST(BIGINT)` (one per value-type), `labels MAP(VARCHAR,VARCHAR)`, `frame LIST(STRUCT(function, filename, line, address))` |
| **Samples (raw)** | `pprof.samples(src)` → `sample_id`, `location_ids LIST(UBIGINT)`, `value LIST(BIGINT)`, `labels MAP` |
| **Functions** | `pprof.functions(src)` → `function_id`, `name`, `system_name`, `filename`, `start_line` |
| **Locations** | `pprof.locations(src)` → `location_id`, `address`, `mapping_id`, `lines LIST(STRUCT(function_id, line))` |
| **Mappings** | `pprof.mappings(src)` → `mapping_id`, `memory_start`, `memory_limit`, `file_offset`, `filename`, `build_id` |
| **Metadata** | `pprof.meta(src)` → one row: `sample_types LIST(STRUCT(type, unit))`, `period`, `period_type`, `duration_nanos`, `time_nanos`, `default_sample_type` |
| **Version** | `pprof.pprof_version()` → the worker's version string |

Every table also appends two trailing columns: **`file`** (the source path, NULL for a BLOB
input) and **`error`** (NULL on success).

### Conventions

- **`pprof.stacks` is the 90% view:** pre-resolved frames + per-value-type values in one row,
  so a flamegraph diff is a `GROUP BY frame … SUM(value)` — no manual location/function join.
  Frames are leaf-first; inlined frames are expanded in place; an unsymbolized native frame has
  a NULL `function`/`filename` but keeps its `address`.
- **Multi-value profiles** (e.g. `alloc_objects` + `alloc_space`) keep `value` as a LIST aligned
  to `meta.sample_types`; callers index the type they want (`value[2]`).
- **Id passthrough.** `samples`, `functions`, `locations`, and `mappings` emit the profile's
  *original* ids verbatim, so they join back together:
  `samples.location_ids[1] → locations.location_id`, `locations.lines[].function_id →
  functions.function_id`, `locations.mapping_id → mappings.mapping_id`.
- **`build_id` on mappings** is emitted verbatim so the addresses of unresolved native frames
  flow to `vgi-symbols` for addr→symbol resolution (the symbolication loop).
- **Per-file error capture.** Each file is decoded independently — a missing, zero-byte, or
  malformed profile yields one **error row** (every data column NULL, `file` + `error` set)
  rather than aborting the scan. Filter with `WHERE error IS NULL`.

## Build & run

```sh
cargo build --release                 # builds crates/pprof-{core,worker}
# DuckDB:
#   ATTACH 'pprof' (TYPE vgi, LOCATION './target/release/pprof-worker');
```

The worker is a standalone binary DuckDB launches and talks to over Apache Arrow IPC. It also
speaks HTTP (`pprof-worker --http`) and a Unix socket (`pprof-worker --unix <sock>`).

## Architecture

Two crates:

- **`pprof-core`** — the pure decode/flatten engine: gunzip (`flate2`) + a `prost`-decoded
  `profile.proto` → plain Rust row structs. No Arrow or VGI dependency, so all decode
  correctness is unit-tested directly. The `profile.proto` schema (Google pprof, Apache-2.0) is
  **vendored** under `crates/pprof-core/proto/` and compiled with `prost-build` (a hermetic
  `protoc` is supplied by `protoc-bin-vendored`, so no system `protoc` is needed).
- **`pprof-worker`** — a thin Arrow adapter: `source` resolves the overloaded `src` argument
  with per-file error capture, `arrow_build` turns the row structs into Arrow columns, and
  `table`/`scalar` register the seven functions.

## Testing

```sh
cargo test                # core golden fixtures (one per producer) + proptest no-panic fuzzing
./run_tests.sh            # haybarn SQLLogic E2E against the built worker (subprocess transport)
TRANSPORT=http ./run_tests.sh    # …or http / unix
```

Golden fixtures are produced by `data/generate_fixtures.go` with the canonical
`google/pprof/profile` library — one per producer shape (Go CPU, Go heap, a multi-value alloc
profile, and an unsymbolized native profile with build ids) — and committed under `data/`.
Regenerate with `cd data && go run generate_fixtures.go`.

## Scope / non-goals

**v1:** decode the standard pprof proto (CPU / heap / alloc / mutex / contention — they share the
schema). **Non-goals:** symbolication (→ `vgi-symbols`), flamegraph *rendering*, writing pprof.
Distinct format/library from `vgi-perf` (Linux `perf.data`); the two ship in one
observability / CI-regression bundle but are not merged.

## License

MIT — see [LICENSE](LICENSE). The vendored `profile.proto` is Apache-2.0 (Google).
Copyright 2026 Query Farm LLC — https://query.farm
