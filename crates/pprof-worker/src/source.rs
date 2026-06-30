//! Resolve the overloaded `src` argument of every pprof table function into a
//! list of decoded profiles, applying **per-file error capture**.
//!
//! `src` is type `any` so DuckDB hands it to us as whatever the caller passed:
//!
//! - a `VARCHAR` path — read & decode one file; the path may be a glob
//!   (`'/profiles/*.pb.gz'`) which expands to every match in sorted order;
//! - a `LIST(VARCHAR)` — several paths/globs, decoded in order;
//! - a `BLOB` — inline profile bytes (one source, no filename).
//!
//! Each resolved file is decoded independently: a missing, zero-byte, or
//! malformed profile becomes a [`DecodeUnit`] whose `result` is `Err(message)`,
//! which the producers render as a single **error row** (every data column NULL,
//! `file` = the path, `error` = the message) instead of aborting the whole scan.

use arrow_array::cast::AsArray;
use arrow_array::Array;
use vgi::arguments::Arguments;
use vgi_rpc::{Result, RpcError};

use pprof_core::Decoded;

/// One decode attempt: the source label (a path, or `None` for inline bytes)
/// and either the decoded profile or a captured error message.
pub struct DecodeUnit {
    pub file: Option<String>,
    pub result: std::result::Result<Decoded, String>,
}

fn ve(msg: impl Into<String>) -> RpcError {
    RpcError::value_error(msg.into())
}

/// Does this path spec contain glob meta-characters?
fn is_glob(spec: &str) -> bool {
    spec.contains(['*', '?', '['])
}

/// Expand one path spec to concrete files: a glob expands to sorted matches; a
/// literal path is returned as-is (its existence is checked at read time, so a
/// missing literal yields an error row rather than vanishing).
fn expand(spec: &str) -> Vec<String> {
    if is_glob(spec) {
        match glob::glob(spec) {
            Ok(paths) => {
                let mut out: Vec<String> = paths
                    .flatten()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                out.sort();
                out
            }
            // A malformed glob pattern: treat it as a literal so the error
            // surfaces as a (not-found) error row.
            Err(_) => vec![spec.to_string()],
        }
    } else {
        vec![spec.to_string()]
    }
}

/// Read the `src` argument at `pos` as a list of path specs, or `None` when the
/// argument is a `BLOB` (handled separately by the caller).
fn path_specs(args: &Arguments, pos: usize) -> Result<Option<Vec<String>>> {
    // Single VARCHAR — the common case.
    if let Some(s) = args.const_str(pos) {
        return Ok(Some(vec![s]));
    }
    let Some(arr) = args.arg(pos) else {
        return Err(ve(
            "a profile source (path, list of paths, or BLOB) is required",
        ));
    };
    // Inline bytes: the BLOB path is handled by the caller, signal with None.
    if matches!(
        arr.data_type(),
        arrow_schema::DataType::Binary
            | arrow_schema::DataType::LargeBinary
            | arrow_schema::DataType::BinaryView
    ) {
        return Ok(None);
    }
    // LIST(VARCHAR): the 1-row positional arg is a list; read its elements.
    let elems = if let Some(l) = arr.as_list_opt::<i32>() {
        l.value(0)
    } else if let Some(l) = arr.as_list_opt::<i64>() {
        l.value(0)
    } else {
        return Err(ve(
            "profile source must be a VARCHAR path, a LIST(VARCHAR), or a BLOB",
        ));
    };
    let mut out = Vec::with_capacity(elems.len());
    if let Some(s) = elems.as_string_opt::<i32>() {
        for i in 0..s.len() {
            if s.is_valid(i) {
                out.push(s.value(i).to_string());
            }
        }
    } else if let Some(s) = elems.as_string_opt::<i64>() {
        for i in 0..s.len() {
            if s.is_valid(i) {
                out.push(s.value(i).to_string());
            }
        }
    } else {
        return Err(ve("path list elements must be VARCHAR"));
    }
    Ok(Some(out))
}

/// Read the inline `BLOB` bytes of the `src` argument at `pos`, if it is one.
fn blob_bytes(args: &Arguments, pos: usize) -> Option<Vec<u8>> {
    let arr = args.arg(pos)?;
    match arr.data_type() {
        arrow_schema::DataType::Binary => {
            let a = arr.as_binary::<i32>();
            a.is_valid(0).then(|| a.value(0).to_vec())
        }
        arrow_schema::DataType::LargeBinary => {
            let a = arr.as_binary::<i64>();
            a.is_valid(0).then(|| a.value(0).to_vec())
        }
        arrow_schema::DataType::BinaryView => {
            let a = arr.as_binary_view();
            a.is_valid(0).then(|| a.value(0).to_vec())
        }
        _ => None,
    }
}

/// Resolve `src` (arg `pos`) into one [`DecodeUnit`] per source, decoding each
/// independently so a bad file is captured, not fatal.
pub fn resolve(args: &Arguments, pos: usize) -> Result<Vec<DecodeUnit>> {
    match path_specs(args, pos)? {
        None => {
            // BLOB source.
            let bytes =
                blob_bytes(args, pos).ok_or_else(|| ve("profile source BLOB was NULL or empty"))?;
            Ok(vec![DecodeUnit {
                file: None,
                result: Decoded::from_bytes(&bytes).map_err(|e| e.to_string()),
            }])
        }
        Some(specs) => {
            let mut units = Vec::new();
            for spec in &specs {
                for path in expand(spec) {
                    let result = std::fs::read(&path)
                        .map_err(|e| format!("read {path}: {e}"))
                        .and_then(|bytes| Decoded::from_bytes(&bytes).map_err(|e| e.to_string()));
                    units.push(DecodeUnit {
                        file: Some(path),
                        result,
                    });
                }
            }
            Ok(units)
        }
    }
}

/// The shared `src` [`vgi::ArgSpec`] used by every pprof table function.
pub fn src_arg_spec() -> vgi::ArgSpec {
    vgi::ArgSpec::const_arg(
        "src",
        0,
        "any",
        "The profile source: a VARCHAR path to a pprof file (gzip-wrapped or raw \
         `profile.proto`), which may be a glob like '/profiles/*.pb.gz' (matches are read in \
         sorted order); a LIST(VARCHAR) of paths/globs; or a BLOB of profile bytes read inline. \
         Each file is decoded independently — a missing, empty, or malformed profile yields one \
         error row (data columns NULL, `file` set, `error` set) rather than failing the scan.",
    )
}
