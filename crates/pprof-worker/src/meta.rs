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
/// that `vgi-lint simulate` runs. Each `(name, prompt, reference_sql)` triple
/// becomes a task object; the `prompt` is shown to the simulated analyst while
/// `reference_sql` (the canonical solution) is hidden and re-run live to grade.
pub fn agent_test_tasks_json(tasks: &[(&str, &str, &str)]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }
    let items: Vec<String> = tasks
        .iter()
        .map(|(name, prompt, reference_sql)| {
            format!(
                "{{\"name\":\"{}\",\"prompt\":\"{}\",\"reference_sql\":\"{}\"}}",
                esc(name),
                esc(prompt),
                esc(reference_sql)
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
// so an object's headline example can decode a profile from a BLOB literal with
// no file on disk — the example is self-contained and copy-paste runnable
// anywhere (`pprof.main.<fn>(from_base64('<...>'))`), and it materializes rows
// under the linter's `--execute` gate (VGI901/902/911) without any fixtures in
// the sandbox. These are byte-identical to data/go_cpu.pb.gz, data/go_heap.pb.gz,
// and data/native.pb.gz (the same profiles the on-disk E2E suite decodes).

/// Base64 of a small Go CPU profile (`value[1]` = samples/count, `value[2]` =
/// cpu/nanoseconds).
pub const GO_CPU_B64: &str = "H4sIAAAAAAAA/yzQPU7rQBAH8Lcfjidr63kUIbAokOXKchFLW3EECgpqKoyJQkTiXWUT0U7HBRAtCCGgocgN0nMFKigouATaFc1/Zn4z1SgJDLmSIFCOUiUEZ3iAdPPyf5SA30W4vdvwfQSGRMRzoi2vokNWZsCQ5fRAvPR3aZkB9/AYgONumYHw8BRAYF1mID08B5AY1woYDvJBEVeqVsARciiG1U6tQKDKVTGsqlqBxCRPirSK9D8du3Zh5xOno86s+5UWnV3rpG974yad6S+cHjVrt2zmpmvnzfmsb1pr9XDRzvqxDx2Hdmr+7Nosr3Ts01sa7NKsjo2xWoXJtks30RDKeGqOiF43X/ff73sndPvzIU7D687o7VNa8RsAAP//CRIJN04BAAA=";

/// Base64 of a small Go heap profile (multi-value: alloc/inuse × objects/space).
pub const GO_HEAP_B64: &str = "H4sIAAAAAAAA/zyMMU6EQBSGndlh9+0A+rKJES0MUhEKSV4sLI2VZ7DRAUeDIhCBwu7dRGNl6w3sPYaF1zAMxmbyzf++fFqBQKkVLFBpBZ7jJarNNkgQG7hhPkuZ9w8QBDKzjJg/Zbo6FckOCBQRv7JMpkiQ7IKchjc3LPAkUSDxKNMgECKI16nONEjUkY799DDTsMAgCmI/Jdqi0NR1W161xb0th568sh2bgfx57TtTWvKK58H2FFbN2Nt/05+/s7LJx/4pr9vS1HlRNbnpOlo/mqo5nh5aObxrKXTg4mawBI6mQ/AnP9jz8ZY8V71gfv/4fvn52rtUEKK6Zo675W8AAAD//wt6pw46AQAA";

/// Base64 of a small unsymbolized native profile whose mappings carry build ids.
pub const NATIVE_B64: &str = "H4sIAAAAAAAA/+Ji4WAUYOJi4WAWYBHi5mDkYBLgEWiYM8dSiJuDmYNRgF2gYfcGRSkhDkaBhoaGFawSDQ0tK1g1WA3YpMQ4mEBiDQ0P5CUaGg6AaA12Aw4lLg5GAUaJBR0NK1iVeDiYBJgktiwBq1Li4mAWYJRYPbFhBasRgxF7cWJuQU5qsRFrcn5pXokRc3JBqRF3XmJefnFqcn5eSrGRgH5+QYl+cVmyflJmHog24kk0TDJKNk4xSTVNMzOS18/JTNKvsDCLNzPRzcnMK63QTc8rBQkm6xXn65kZ8aSkJqYkpaamJSempXo0NKzY+Hj+89PiAQ2ntr5jjAJ7OqFh7SOWAgZAAAAA//9plBkzCAEAAA==";
