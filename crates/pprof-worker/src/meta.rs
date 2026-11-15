//! Shared helpers for the per-object discovery/description metadata that the
//! `vgi-lint` strict profile expects on **every** function and table.
//!
//! Each function/table surfaces these in its `FunctionMetadata.tags`:
//! - `vgi.title` (VGI124)            — human-friendly display name
//! - `vgi.doc_llm` (VGI112)          — concise prose aimed at LLMs
//! - `vgi.doc_md` (VGI113)           — short Markdown description
//! - `vgi.keywords` (VGI126/VGI138)  — a JSON array of search terms/synonyms
//!
//! Per-object `vgi.source_url` is intentionally NOT emitted here: `vgi.source_url`
//! belongs on the catalog object only (VGI139). The catalog's `source_url` field
//! already points at the repo.

/// Encode comma-separated keywords as the JSON array of strings that
/// `vgi.keywords` requires (VGI138). Each term is trimmed and empty terms are
/// dropped; the result is e.g. `["fixed-width","unpack","copybook"]`.
pub fn keywords_json(keywords: &str) -> String {
    let items: Vec<String> = keywords
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        // JSON-escape each keyword (covers quotes/backslashes) by emitting a
        // one-element array and stripping the surrounding brackets.
        .map(|k| {
            let escaped = k.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build a table function's example set from ONE list of `(description, sql)`
/// pairs, returning both the native `vgi::FunctionExample` vec (surfaced into the
/// `duckdb_functions().examples` column) and the described `vgi.example_queries`
/// tag JSON. Because both carriers are generated from the same `sql` strings, the
/// linter's dedup keeps the described copy — the native column drops per-example
/// descriptions, so without the tag VGI515 would flag every example as
/// undescribed.
pub fn described_examples(pairs: Vec<(String, String)>) -> (Vec<vgi::FunctionExample>, String) {
    let examples = pairs
        .iter()
        .map(|(description, sql)| vgi::FunctionExample {
            sql: sql.clone(),
            description: description.clone(),
            expected_output: None,
        })
        .collect();
    (examples, example_queries_json(&pairs))
}

/// Encode `(description, sql)` pairs as the described `vgi.example_queries` JSON
/// array of `{"description","sql"}` objects (VGI502/VGI515).
pub fn example_queries_json(pairs: &[(String, String)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
            .replace('\t', " ")
    }
    let items: Vec<String> = pairs
        .iter()
        .map(|(description, sql)| {
            format!(
                "{{\"description\":\"{}\",\"sql\":\"{}\"}}",
                esc(description),
                esc(sql)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build the `vgi.agent_test_tasks` JSON value: a fixed suite of analyst tasks
/// that `vgi-lint simulate` runs. Each `(name, prompt)` pair becomes a task
/// object shown to the simulated analyst. The graders (`reference_sql`, the
/// canonical solution re-run live to grade) are private and live in the
/// `vgi-agent-tests.yaml` sidecar at the repo root, keyed by task name (VGI416).
pub fn agent_test_tasks_json(tasks: &[(&str, &str)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }
    let items: Vec<String> = tasks
        .iter()
        .map(|(name, prompt)| {
            format!(
                "{{\"name\":\"{}\",\"prompt\":\"{}\"}}",
                esc(name),
                esc(prompt)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Build the `vgi.result_columns_schema` JSON value: a JSON array of
/// `{name, type, description}` objects, one per returned column, in order. This
/// is the structured, lintable declaration of a table function's static result
/// shape (VGI307/VGI321-323) that replaced the retired free-form
/// `vgi.result_columns_md` (VGI414). Each `type` must be a real DuckDB type and
/// each `description` non-blank.
pub fn result_columns_schema(columns: &[(&str, &str, &str)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let items: Vec<String> = columns
        .iter()
        .map(|(name, ty, description)| {
            format!(
                "{{\"name\":\"{}\",\"type\":\"{}\",\"description\":\"{}\"}}",
                esc(name),
                esc(ty),
                esc(description)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The two trailing provenance/error columns every table function appends, as
/// `(name, type, description)` triples for [`result_columns_schema`]. Kept here
/// so each table declares them identically without repetition.
pub fn trailing_result_columns() -> [(&'static str, &'static str, &'static str); 2] {
    [
        (
            "file",
            "VARCHAR",
            "Source path the row came from (NULL when the profile was passed as inline BLOB bytes).",
        ),
        (
            "error",
            "VARCHAR",
            "NULL on success; otherwise this is an error row for a file that could not be read or \
             decoded, with every data column NULL and the failure message here.",
        ),
    ]
}

/// Build the standard per-object discovery/description tags
/// (`vgi.title`, `vgi.doc_llm`, `vgi.doc_md`, `vgi.keywords`) plus the object's
/// primary `vgi.category` — the name of one of the categories declared in the
/// schema's `vgi.categories` registry (VGI409/VGI411), which drives navigation,
/// listing sections, and SEO descriptions.
pub fn object_tags(
    title: &str,
    description_llm: &str,
    description_md: &str,
    keywords: &str,
    category: &str,
) -> Vec<(String, String)> {
    vec![
        ("vgi.title".to_string(), title.to_string()),
        ("vgi.doc_llm".to_string(), description_llm.to_string()),
        ("vgi.doc_md".to_string(), description_md.to_string()),
        ("vgi.keywords".to_string(), keywords_json(keywords)),
        ("vgi.category".to_string(), category.to_string()),
    ]
}

// Tiny, real pprof fixtures (gzip-wrapped `profile.proto`) base64-encoded inline
// so every example can decode a profile from a BLOB literal with no file on disk:
// the example is self-contained and copy-paste runnable anywhere
// (`pprof.main.<fn>(from_base64('<...>'))`), and it materializes rows under the
// linter's `--execute` gate (VGI901/902/911/913) for any user, not only inside
// this repo (a relative `data/...` path resolves nowhere else — VGI516). These are
// byte-identical to the committed `data/*.pb.gz` fixtures (the same profiles the
// on-disk E2E suite decodes; `fixtures_match_committed_files` pins that).
//
// Each fixture is a macro (expanding to the literal) so examples can splice it
// into `concat!` at compile time, plus a `&str` const for `format!` call sites.

/// Base64 of `data/go_cpu.pb.gz`: a small Go CPU profile (`value[1]` =
/// samples/count, `value[2]` = cpu/nanoseconds).
macro_rules! go_cpu_b64 {
    () => {
        "H4sIAAAAAAAA/yzQPU7rQBAH8Lcfjidr63kUIbAokOXKchFLW3EECgpqKoyJQkTiXWUT0U7HBRAtCCGgocgN0nMFKigouATaFc1/Zn4z1SgJDLmSIFCOUiUEZ3iAdPPyf5SA30W4vdvwfQSGRMRzoi2vokNWZsCQ5fRAvPR3aZkB9/AYgONumYHw8BRAYF1mID08B5AY1woYDvJBEVeqVsARciiG1U6tQKDKVTGsqlqBxCRPirSK9D8du3Zh5xOno86s+5UWnV3rpG974yad6S+cHjVrt2zmpmvnzfmsb1pr9XDRzvqxDx2Hdmr+7Nosr3Ts01sa7NKsjo2xWoXJtks30RDKeGqOiF43X/ff73sndPvzIU7D687o7VNa8RsAAP//CRIJN04BAAA="
    };
}
pub(crate) use go_cpu_b64;
pub const GO_CPU_B64: &str = go_cpu_b64!();

/// Base64 of `data/go_cpu2.pb.gz`: a second Go CPU profile (a "candidate" run of
/// the same program as [`GO_CPU_B64`]).
macro_rules! go_cpu2_b64 {
    () => {
        "H4sIAAAAAAAA/yzQvU7DQAwHcO4jjXuJFKtCEDGgKFOUoZFu4hEYGJiZCKEqFW3u1GvF6ocAVhBCiIWhb9CdV2CCgYGZHd2J5W/7Z09WEhhyJUGgHKVKCM7wEOnutx4l4HcRbu83/ACBIRHxnGjLq+iIlRkwZDk9Ei/9XVpmwD08BeC4V2YgPDwHEFiXGUgPLwEkxrUChoN8UMSVqhVwhByKYbVbKxCoclUMq6pWIDHJkyKtIr2jY9cu7HzidNSZdb/SorNrnfRtb9ykM/2l06Nm7ZbN3HTtvLmY9U1rrR4u2lk/9qHj0E7Nv92Y5bWOfXpLg12Z1YkxVqsw2XbpJhpCGU/NMdHr5uvh+33/lG5/PsRZeN05vX1KK/4CAAD//2pKKrROAQAA"
    };
}
pub(crate) use go_cpu2_b64;

/// Base64 of `data/go_heap.pb.gz`: a small Go heap profile (multi-value:
/// alloc/inuse × objects/space).
macro_rules! go_heap_b64 {
    () => {
        "H4sIAAAAAAAA/zyMMU6EQBSGndlh9+0A+rKJES0MUhEKSV4sLI2VZ7DRAUeDIhCBwu7dRGNl6w3sPYaF1zAMxmbyzf++fFqBQKkVLFBpBZ7jJarNNkgQG7hhPkuZ9w8QBDKzjJg/Zbo6FckOCBQRv7JMpkiQ7IKchjc3LPAkUSDxKNMgECKI16nONEjUkY799DDTsMAgCmI/Jdqi0NR1W161xb0th568sh2bgfx57TtTWvKK58H2FFbN2Nt/05+/s7LJx/4pr9vS1HlRNbnpOlo/mqo5nh5aObxrKXTg4mawBI6mQ/AnP9jz8ZY8V71gfv/4fvn52rtUEKK6Zo675W8AAAD//wt6pw46AQAA"
    };
}
pub(crate) use go_heap_b64;
pub const GO_HEAP_B64: &str = go_heap_b64!();

/// Base64 of `data/native.pb.gz`: a small unsymbolized native profile whose
/// mappings carry build ids.
macro_rules! native_b64 {
    () => {
        "H4sIAAAAAAAA/+Ji4WAUYOJi4WAWYBHi5mDkYBLgEWiYM8dSiJuDmYNRgF2gYfcGRSkhDkaBhoaGFawSDQ0tK1g1WA3YpMQ4mEBiDQ0P5CUaGg6AaA12Aw4lLg5GAUaJBR0NK1iVeDiYBJgktiwBq1Li4mAWYJRYPbFhBasRgxF7cWJuQU5qsRFrcn5pXokRc3JBqRF3XmJefnFqcn5eSrGRgH5+QYl+cVmyflJmHog24kk0TDJKNk4xSTVNMzOS18/JTNKvsDCLNzPRzcnMK63QTc8rBQkm6xXn65kZ8aSkJqYkpaamJSempXo0NKzY+Hj+89PiAQ2ntr5jjAJ7OqFh7SOWAgZAAAAA//9plBkzCAEAAA=="
    };
}
pub(crate) use native_b64;
pub const NATIVE_B64: &str = native_b64!();

/// Base64 of `data/alloc.pb.gz`: a small allocation profile whose samples carry
/// labels (e.g. a heap profile's size_class tag).
macro_rules! alloc_b64 {
    () => {
        "H4sIAAAAAAAA/yyNMU7DQBBF2ckaxuugjNJgpbJcWSkIGiEELRVnoAn2skEgy468ThBUcxNaWm4Q0XIMCq6BvEn1Zt7X1zcaFYHROCI9nSKgIiESEZhpjOh4RoMRgVRkB8XJtconqEilshPIh3KTTxAG8R0EUDw3qAhTzOLicW4QyKQmSwrkIz4t67q1y7Z6cbb3HNl20/Sc7K1fl9ZxVL31zrPxz+9uaevSe9aXFzdXnCx8t114121dx+M9zwM4PnxPLdPhbNzr7Wa1ch3HVeAQRmHiTuTz6/fj7+fsXuOY9INIth79BwAA//+aqrQCCgEAAA=="
    };
}
pub(crate) use alloc_b64;

// Composed examples: each combines two or more of the worker's own tables in one
// query a perf engineer would actually write. Every SQL is defined ONCE here and
// referenced from the schema-level `vgi.example_queries` list (lib.rs), from the
// example list of the table that drives the query (within its first five, which
// is all the vgi-lint agent simulator shows), and from that table's
// `vgi.executable_examples`. They decode the inline fixtures above, so they are
// offline, deterministic, and runnable anywhere; test/sql/composed.test pins
// their rows.

/// `meta` + `stacks`: pick the profile's value slot by name from
/// `meta.default_sample_type` instead of hard-coding `value[N]`, so the same
/// query aggregates CPU and heap profiles alike.
pub const EX_DEFAULT_TYPE_BY_LEAF: (&str, &str) = (
    "Total a heap profile's default sample type (inuse_space) per leaf function, finding the \
     value slot by name from pprof.main.meta instead of hard-coding value[N], so the same query \
     works unchanged on a CPU profile (where it totals cpu).",
    concat!(
        "WITH m AS (SELECT default_sample_type AS sample_type, \
         list_position([t.type FOR t IN sample_types], default_sample_type) AS slot \
         FROM pprof.main.meta(from_base64('",
        go_heap_b64!(),
        "')) WHERE error IS NULL) \
         SELECT m.sample_type, s.frame[1].function AS fn, sum(s.value[m.slot]) AS total \
         FROM pprof.main.stacks(from_base64('",
        go_heap_b64!(),
        "')) s, m WHERE s.error IS NULL GROUP BY ALL ORDER BY total DESC, fn"
    ),
);

/// `samples` + `locations` + `mappings`: the hottest unsymbolized leaf
/// addresses as (build_id, binary-relative offset) pairs — exactly what a
/// downstream symbolizer needs.
pub const EX_HOT_NATIVE_OFFSETS: (&str, &str) = (
    "Hottest unsymbolized leaf addresses as symbolizer-ready (binary, build_id, offset in the \
     binary) rows, weighted by CPU time: each sample's leaf location id is joined to \
     pprof.main.locations and on to pprof.main.mappings.",
    concat!(
        "SELECT m.filename AS binary_path, m.build_id, \
         printf('0x%x', l.address - m.memory_start + m.file_offset) AS binary_offset, \
         sum(s.value[2]) AS cpu_ns \
         FROM pprof.main.samples(from_base64('",
        native_b64!(),
        "')) s JOIN pprof.main.locations(from_base64('",
        native_b64!(),
        "')) l ON l.location_id = s.location_ids[1] \
         JOIN pprof.main.mappings(from_base64('",
        native_b64!(),
        "')) m ON m.mapping_id = l.mapping_id \
         WHERE s.error IS NULL GROUP BY ALL ORDER BY cpu_ns DESC, binary_offset"
    ),
);

/// `locations` + `functions`: expand each multi-entry line table into the
/// chain of functions the compiler inlined into one instruction address.
pub const EX_INLINED_FRAMES: (&str, &str) = (
    "Show what the compiler inlined: for every location whose line table has more than one \
     entry, name each function folded into that address (depth 1 = innermost) by joining the \
     line table to pprof.main.functions.",
    concat!(
        "WITH l AS (SELECT location_id, address, unnest(lines) AS ln, \
         generate_subscripts(lines, 1) AS depth \
         FROM pprof.main.locations(from_base64('",
        go_heap_b64!(),
        "')) WHERE error IS NULL AND len(lines) > 1) \
         SELECT l.location_id, l.address, l.depth, f.name AS function_name, f.filename, \
         l.ln.line AS line \
         FROM l JOIN pprof.main.functions(from_base64('",
        go_heap_b64!(),
        "')) f ON f.function_id = l.ln.function_id \
         ORDER BY l.location_id, l.depth"
    ),
);

/// Top self-time functions summed across two CPU profiles, each passed inline.
pub const EX_TOP_ACROSS_PROFILES: (&str, &str) = (
    "Top self-time functions summed across two CPU profiles (a baseline and a candidate run, \
     passed inline as BLOBs; with profiles on disk, pass a path glob or a list of paths).",
    concat!(
        "SELECT fn, sum(cpu_ns) AS cpu_ns FROM (SELECT frame[1].function AS fn, value[2] AS cpu_ns \
         FROM pprof.main.stacks(from_base64('",
        go_cpu_b64!(),
        "')) WHERE error IS NULL UNION ALL SELECT frame[1].function, value[2] \
         FROM pprof.main.stacks(from_base64('",
        go_cpu2_b64!(),
        "')) WHERE error IS NULL) GROUP BY fn ORDER BY cpu_ns DESC, fn LIMIT 20"
    ),
);

/// A composed example as the owned `(description, sql)` pair the example
/// helpers take.
pub fn example_pair(example: (&str, &str)) -> (String, String) {
    (example.0.to_string(), example.1.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inline fixtures must stay byte-identical to the committed `data/`
    /// files the E2E suite decodes.
    #[test]
    fn fixtures_match_committed_files() {
        // Minimal standard-alphabet base64 encoder (no extra dependency).
        fn encode(bytes: &[u8]) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let n = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
                for i in 0..4 {
                    if i <= chunk.len() {
                        out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
                    } else {
                        out.push('=');
                    }
                }
            }
            out
        }
        let data = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/");
        for (name, b64) in [
            ("go_cpu.pb.gz", go_cpu_b64!()),
            ("go_cpu2.pb.gz", go_cpu2_b64!()),
            ("go_heap.pb.gz", go_heap_b64!()),
            ("native.pb.gz", native_b64!()),
            ("alloc.pb.gz", alloc_b64!()),
        ] {
            let on_disk = std::fs::read(format!("{data}{name}")).unwrap();
            assert_eq!(encode(&on_disk), b64, "{name}");
        }
    }

    /// The composed examples feed JSON tags and the lint's per-object lists:
    /// each must stay a single-line, catalog-qualified, ORDER BY'd statement.
    #[test]
    fn composed_examples_are_single_line_qualified_and_ordered() {
        for (description, sql) in [
            EX_DEFAULT_TYPE_BY_LEAF,
            EX_HOT_NATIVE_OFFSETS,
            EX_INLINED_FRAMES,
        ] {
            assert!(
                !sql.contains("data/"),
                "examples must not read local files: {sql}"
            );
            assert!(!description.trim().is_empty());
            assert!(!sql.contains('\n'), "{sql}");
            assert!(sql.contains("ORDER BY"), "{sql}");
            assert!(!sql.contains("SELECT *"), "{sql}");
            let qualified = sql.matches("pprof.main.").count();
            assert!(
                qualified >= 2,
                "composed example must combine 2+ objects: {sql}"
            );
        }
    }
}
