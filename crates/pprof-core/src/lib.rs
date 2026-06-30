//! Pure pprof decoder + flattener.
//!
//! `pprof-core` takes the bytes of a pprof profile — the gzip-wrapped
//! `profile.proto` produced by Go, `gperftools`, Parca, Pyroscope, py-spy, etc.
//! — and flattens the string-table-indexed protobuf graph into plain Rust row
//! structs (one per SQL view the worker exposes). It has **no Arrow or VGI
//! dependency**: all decode/resolution correctness lives here and is unit-tested
//! directly; the `pprof-worker` crate is a thin Arrow adapter on top.
//!
//! ```no_run
//! let bytes = std::fs::read("cpu.pb.gz").unwrap();
//! let prof = pprof_core::Decoded::from_bytes(&bytes).unwrap();
//! for s in prof.stacks() {
//!     // s.frames[0] is the leaf; s.value is aligned to prof.meta().sample_types
//! }
//! ```
//!
//! ## The pprof model, briefly
//! A profile is a set of [`proto::Sample`]s. Each sample references a sequence of
//! location ids (leaf first), each [`proto::Location`] belongs to an optional
//! [`proto::Mapping`] and carries zero or more [`proto::Line`]s (inlined frames,
//! innermost first), and each line names a [`proto::Function`]. Strings (function
//! names, filenames, units, build ids) are interned in `string_table`; every
//! other message holds *indices* into it. The flatteners below resolve those
//! indices and pre-join the location/function graph so callers get rows, not a
//! pointer chase.
//!
//! ## Id passthrough
//! Every flattener emits the profile's **original** ids verbatim
//! (`function_id`, `location_id`, `mapping_id`) so the row views join back
//! together (and so unresolved native frames carry their `mapping.build_id`
//! onward to a symbolizer such as `vgi-symbols`). Ids are never renumbered.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;

/// The prost-generated `profile.proto` types (`package perftools.profiles`).
/// Vendored proto is Apache-2.0; see `proto/profile.proto`.
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/perftools.profiles.rs"));
}

/// Failure to turn input bytes into a profile.
#[derive(Debug)]
pub enum PprofError {
    /// Zero-length input — nothing to decode.
    Empty,
    /// The gzip wrapper could not be inflated.
    Gunzip(std::io::Error),
    /// The protobuf body was not a valid `profile.proto` message.
    Decode(prost::DecodeError),
}

impl fmt::Display for PprofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PprofError::Empty => write!(f, "empty profile (zero bytes)"),
            PprofError::Gunzip(e) => write!(f, "gzip decompression failed: {e}"),
            PprofError::Decode(e) => write!(f, "pprof protobuf decode failed: {e}"),
        }
    }
}

impl std::error::Error for PprofError {}

/// A resolved value type: the `type` and `unit` strings of a `ValueType` (e.g.
/// `("cpu", "nanoseconds")` or `("alloc_space", "bytes")`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueType {
    pub r#type: String,
    pub unit: String,
}

/// One pre-resolved frame in a flattened stack: a symbolized function (when the
/// profile carries debug info) plus the raw instruction `address` (always
/// present) so unsymbolized native frames still flow onward to a symbolizer.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Human-readable function name, or `None` for an unsymbolized frame.
    pub function: Option<String>,
    /// Source file of the function, or `None` when unknown.
    pub filename: Option<String>,
    /// Source line, or `None` when unknown.
    pub line: Option<i64>,
    /// Instruction address within the mapping (0 when unavailable).
    pub address: u64,
}

/// A `(function_id, line)` pair from a location's line table (id passthrough).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRef {
    pub function_id: u64,
    pub line: i64,
}

/// The headline flattened-stack row: per-value-type values + pre-resolved
/// frames, so a flamegraph diff is a `GROUP BY frame … value` with no join.
#[derive(Debug, Clone, PartialEq)]
pub struct StackRow {
    /// 1-based index of the sample within the profile (joins to [`SampleRow`]).
    pub sample_id: i64,
    /// One value per `meta().sample_types` entry, in the same order.
    pub value: Vec<i64>,
    /// Sample labels as `(key, value)` pairs, keys de-duplicated (first wins).
    pub labels: Vec<(String, String)>,
    /// Frames, leaf first (innermost inlined frame first within a location).
    pub frames: Vec<Frame>,
}

/// A raw sample row: location ids + values + labels, no resolution applied.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleRow {
    pub sample_id: i64,
    pub location_ids: Vec<u64>,
    pub value: Vec<i64>,
    pub labels: Vec<(String, String)>,
}

/// A function-table row (ids passed through verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRow {
    pub function_id: u64,
    pub name: Option<String>,
    pub system_name: Option<String>,
    pub filename: Option<String>,
    pub start_line: i64,
}

/// A location-table row (ids passed through verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    pub location_id: u64,
    pub address: u64,
    pub mapping_id: u64,
    pub lines: Vec<LineRef>,
}

/// A mapping-table row. `build_id` is emitted verbatim so unresolved native
/// frames can be symbolized downstream (the `vgi-symbols` loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRow {
    pub mapping_id: u64,
    pub memory_start: u64,
    pub memory_limit: u64,
    pub file_offset: u64,
    pub filename: Option<String>,
    pub build_id: Option<String>,
}

/// The single metadata row: sample value-types, sampling period, profile
/// duration/time, and the preferred (default) sample type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub sample_types: Vec<ValueType>,
    pub period: i64,
    pub period_type: Option<ValueType>,
    pub duration_nanos: i64,
    pub time_nanos: i64,
    pub default_sample_type: Option<String>,
}

/// A decoded profile plus id→index maps for O(1) location/function resolution.
pub struct Decoded {
    profile: proto::Profile,
    loc_by_id: HashMap<u64, usize>,
    fn_by_id: HashMap<u64, usize>,
}

impl Decoded {
    /// Decode profile `bytes` (gzip-wrapped or raw protobuf) into a [`Decoded`].
    ///
    /// The on-disk pprof format is gzip-compressed; we sniff the gzip magic
    /// (`1f 8b`) and inflate when present, otherwise decode the bytes as a raw
    /// `profile.proto` message (some producers/tests emit uncompressed). Never
    /// panics: malformed input returns [`PprofError`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PprofError> {
        if bytes.is_empty() {
            return Err(PprofError::Empty);
        }
        let raw = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(bytes)
                .read_to_end(&mut out)
                .map_err(PprofError::Gunzip)?;
            out
        } else {
            bytes.to_vec()
        };
        let profile = <proto::Profile as prost::Message>::decode(raw.as_slice())
            .map_err(PprofError::Decode)?;

        let loc_by_id = profile
            .location
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        let fn_by_id = profile
            .function
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, i))
            .collect();
        Ok(Decoded {
            profile,
            loc_by_id,
            fn_by_id,
        })
    }

    /// Resolve a `string_table` index to a string, treating index 0 (the
    /// mandatory empty string) and any out-of-range index as `None`.
    fn str(&self, idx: i64) -> Option<&str> {
        if idx <= 0 {
            return None;
        }
        self.profile
            .string_table
            .get(idx as usize)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    fn value_type(&self, vt: &proto::ValueType) -> ValueType {
        ValueType {
            r#type: self.str(vt.r#type).unwrap_or_default().to_string(),
            unit: self.str(vt.unit).unwrap_or_default().to_string(),
        }
    }

    /// Resolve a sample's labels to `(key, value)` string pairs. String labels
    /// resolve through the string table; numeric labels render as the number
    /// (with a unit suffix when present). Keys are de-duplicated keeping the
    /// first occurrence — a DuckDB `MAP` requires unique keys, and pprof itself
    /// strongly discourages multi-value label keys.
    fn labels(&self, sample: &proto::Sample) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::with_capacity(sample.label.len());
        for lbl in &sample.label {
            let key = self.str(lbl.key).unwrap_or_default().to_string();
            let value = if lbl.str != 0 {
                self.str(lbl.str).unwrap_or_default().to_string()
            } else if let Some(unit) = self.str(lbl.num_unit) {
                format!("{} {}", lbl.num, unit)
            } else {
                lbl.num.to_string()
            };
            if !out.iter().any(|(k, _)| k == &key) {
                out.push((key, value));
            }
        }
        out
    }

    /// Expand one location id into its frames (leaf/innermost first). An
    /// unsymbolized location (no line table, or a dangling id) yields a single
    /// address-only frame so the raw address still reaches a symbolizer.
    fn frames_for(&self, location_id: u64, out: &mut Vec<Frame>) {
        let Some(&li) = self.loc_by_id.get(&location_id) else {
            // Dangling location id: emit nothing resolvable but keep the address
            // slot so downstream joins still see the (unknown) location.
            out.push(Frame {
                function: None,
                filename: None,
                line: None,
                address: 0,
            });
            return;
        };
        let loc = &self.profile.location[li];
        if loc.line.is_empty() {
            out.push(Frame {
                function: None,
                filename: None,
                line: None,
                address: loc.address,
            });
            return;
        }
        for line in &loc.line {
            let func = self
                .fn_by_id
                .get(&line.function_id)
                .map(|&fi| &self.profile.function[fi]);
            out.push(Frame {
                function: func.and_then(|f| self.str(f.name)).map(str::to_string),
                filename: func.and_then(|f| self.str(f.filename)).map(str::to_string),
                line: Some(line.line),
                address: loc.address,
            });
        }
    }

    /// The profile metadata row.
    pub fn meta(&self) -> Meta {
        Meta {
            sample_types: self
                .profile
                .sample_type
                .iter()
                .map(|vt| self.value_type(vt))
                .collect(),
            period: self.profile.period,
            period_type: self
                .profile
                .period_type
                .as_ref()
                .map(|vt| self.value_type(vt)),
            duration_nanos: self.profile.duration_nanos,
            time_nanos: self.profile.time_nanos,
            default_sample_type: self
                .str(self.profile.default_sample_type)
                .map(str::to_string),
        }
    }

    /// The flattened-stack rows (the headline view).
    pub fn stacks(&self) -> Vec<StackRow> {
        self.profile
            .sample
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                let mut frames = Vec::new();
                for &loc_id in &sample.location_id {
                    self.frames_for(loc_id, &mut frames);
                }
                StackRow {
                    sample_id: (i as i64) + 1,
                    value: sample.value.clone(),
                    labels: self.labels(sample),
                    frames,
                }
            })
            .collect()
    }

    /// The raw sample rows.
    pub fn samples(&self) -> Vec<SampleRow> {
        self.profile
            .sample
            .iter()
            .enumerate()
            .map(|(i, sample)| SampleRow {
                sample_id: (i as i64) + 1,
                location_ids: sample.location_id.clone(),
                value: sample.value.clone(),
                labels: self.labels(sample),
            })
            .collect()
    }

    /// The function-table rows.
    pub fn functions(&self) -> Vec<FunctionRow> {
        self.profile
            .function
            .iter()
            .map(|f| FunctionRow {
                function_id: f.id,
                name: self.str(f.name).map(str::to_string),
                system_name: self.str(f.system_name).map(str::to_string),
                filename: self.str(f.filename).map(str::to_string),
                start_line: f.start_line,
            })
            .collect()
    }

    /// The location-table rows.
    pub fn locations(&self) -> Vec<LocationRow> {
        self.profile
            .location
            .iter()
            .map(|l| LocationRow {
                location_id: l.id,
                address: l.address,
                mapping_id: l.mapping_id,
                lines: l
                    .line
                    .iter()
                    .map(|ln| LineRef {
                        function_id: ln.function_id,
                        line: ln.line,
                    })
                    .collect(),
            })
            .collect()
    }

    /// The mapping-table rows (`build_id` emitted verbatim for symbolication).
    pub fn mappings(&self) -> Vec<MappingRow> {
        self.profile
            .mapping
            .iter()
            .map(|m| MappingRow {
                mapping_id: m.id,
                memory_start: m.memory_start,
                memory_limit: m.memory_limit,
                file_offset: m.file_offset,
                filename: self.str(m.filename).map(str::to_string),
                build_id: self.str(m.build_id).map(str::to_string),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny in-memory profile: one CPU sample with two inlined frames
    /// in one location, a second unsymbolized location with an address, a
    /// mapping carrying a build id, and a label. Exercises every flattener.
    fn synthetic() -> proto::Profile {
        // string_table[0] must be "".
        let strings = vec![
            "".to_string(),            // 0
            "samples".to_string(),     // 1
            "count".to_string(),       // 2
            "cpu".to_string(),         // 3
            "nanoseconds".to_string(), // 4
            "main".to_string(),        // 5
            "main.go".to_string(),     // 6
            "inlined".to_string(),     // 7
            "thread".to_string(),      // 8 (label key)
            "worker".to_string(),      // 9 (label value)
            "abc123".to_string(),      // 10 (build id)
            "/bin/app".to_string(),    // 11 (mapping filename)
        ];
        proto::Profile {
            sample_type: vec![
                proto::ValueType { r#type: 1, unit: 2 },
                proto::ValueType { r#type: 3, unit: 4 },
            ],
            sample: vec![proto::Sample {
                location_id: vec![1, 2],
                value: vec![10, 2000],
                label: vec![proto::Label {
                    key: 8,
                    str: 9,
                    num: 0,
                    num_unit: 0,
                }],
            }],
            mapping: vec![proto::Mapping {
                id: 1,
                memory_start: 0x400000,
                memory_limit: 0x500000,
                file_offset: 0,
                filename: 11,
                build_id: 10,
                ..Default::default()
            }],
            location: vec![
                proto::Location {
                    id: 1,
                    mapping_id: 1,
                    address: 0x401000,
                    line: vec![
                        proto::Line {
                            function_id: 7,
                            line: 42,
                            column: 0,
                        },
                        proto::Line {
                            function_id: 5,
                            line: 10,
                            column: 0,
                        },
                    ],
                    is_folded: false,
                },
                proto::Location {
                    id: 2,
                    mapping_id: 1,
                    address: 0x402000,
                    line: vec![],
                    is_folded: false,
                },
            ],
            function: vec![
                proto::Function {
                    id: 5,
                    name: 5,
                    system_name: 5,
                    filename: 6,
                    start_line: 1,
                },
                proto::Function {
                    id: 7,
                    name: 7,
                    system_name: 7,
                    filename: 6,
                    start_line: 40,
                },
            ],
            string_table: strings,
            period: 10_000_000,
            period_type: Some(proto::ValueType { r#type: 3, unit: 4 }),
            duration_nanos: 1_000_000_000,
            time_nanos: 1_700_000_000_000_000_000,
            default_sample_type: 3,
            ..Default::default()
        }
    }

    fn gzip(profile: &proto::Profile) -> Vec<u8> {
        use prost::Message;
        let mut body = Vec::new();
        profile.encode(&mut body).unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut enc, &body).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn decodes_gzip_and_raw_identically() {
        let p = synthetic();
        let raw = {
            use prost::Message;
            let mut b = Vec::new();
            p.encode(&mut b).unwrap();
            b
        };
        let from_raw = Decoded::from_bytes(&raw).unwrap();
        let from_gz = Decoded::from_bytes(&gzip(&p)).unwrap();
        assert_eq!(from_raw.stacks(), from_gz.stacks());
    }

    #[test]
    fn meta_resolves_value_types() {
        let d = Decoded::from_bytes(&gzip(&synthetic())).unwrap();
        let m = d.meta();
        assert_eq!(m.sample_types.len(), 2);
        assert_eq!(m.sample_types[0].r#type, "samples");
        assert_eq!(m.sample_types[1].unit, "nanoseconds");
        assert_eq!(m.period, 10_000_000);
        assert_eq!(m.period_type.unwrap().r#type, "cpu");
        assert_eq!(m.default_sample_type.as_deref(), Some("cpu"));
    }

    #[test]
    fn stacks_flatten_inlined_and_unsymbolized_frames() {
        let d = Decoded::from_bytes(&gzip(&synthetic())).unwrap();
        let stacks = d.stacks();
        assert_eq!(stacks.len(), 1);
        let s = &stacks[0];
        assert_eq!(s.sample_id, 1);
        assert_eq!(s.value, vec![10, 2000]);
        assert_eq!(s.labels, vec![("thread".to_string(), "worker".to_string())]);
        // Location 1 has two inlined lines (innermost first), location 2 is
        // unsymbolized (address only) → 3 frames total.
        assert_eq!(s.frames.len(), 3);
        assert_eq!(s.frames[0].function.as_deref(), Some("inlined"));
        assert_eq!(s.frames[0].line, Some(42));
        assert_eq!(s.frames[0].address, 0x401000);
        assert_eq!(s.frames[1].function.as_deref(), Some("main"));
        assert_eq!(s.frames[2].function, None);
        assert_eq!(s.frames[2].address, 0x402000);
    }

    #[test]
    fn tables_pass_ids_through() {
        let d = Decoded::from_bytes(&gzip(&synthetic())).unwrap();
        let funcs = d.functions();
        assert_eq!(funcs.len(), 2);
        assert!(funcs
            .iter()
            .any(|f| f.function_id == 7 && f.name.as_deref() == Some("inlined")));

        let locs = d.locations();
        let l1 = locs.iter().find(|l| l.location_id == 1).unwrap();
        assert_eq!(l1.mapping_id, 1);
        assert_eq!(
            l1.lines,
            vec![
                LineRef {
                    function_id: 7,
                    line: 42
                },
                LineRef {
                    function_id: 5,
                    line: 10
                },
            ]
        );

        let maps = d.mappings();
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].build_id.as_deref(), Some("abc123"));
        assert_eq!(maps[0].filename.as_deref(), Some("/bin/app"));
    }

    #[test]
    fn empty_input_is_an_error_not_a_panic() {
        assert!(matches!(Decoded::from_bytes(&[]), Err(PprofError::Empty)));
    }

    #[test]
    fn garbage_input_is_an_error_not_a_panic() {
        // Random non-gzip, non-proto bytes must return Err, never panic.
        let junk = [0xFFu8; 64];
        assert!(Decoded::from_bytes(&junk).is_err());
    }
}
