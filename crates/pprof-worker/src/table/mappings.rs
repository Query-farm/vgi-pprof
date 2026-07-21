//! `pprof.mappings(src)` — the mapping table: mapping_id → memory range /
//! file_offset / filename / build_id. `build_id` feeds `vgi-symbols`.

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "Mappings with build ids (the addresses that still need symbolizing).",
    "sql": "SELECT mapping_id, filename, build_id FROM pprof.main.mappings('data/native.pb.gz') WHERE error IS NULL AND build_id IS NOT NULL"
  },
  {
    "description": "Every loaded binary/segment in a profile with its address range (memory_start .. memory_limit).",
    "sql": "SELECT mapping_id, filename, memory_start, memory_limit FROM pprof.main.mappings('data/go_cpu.pb.gz') WHERE error IS NULL ORDER BY mapping_id"
  }
]"#;

pub struct Mappings;

pub fn output_schema() -> SchemaRef {
    schema_with_trailing(vec![
        commented(
            "mapping_id",
            DataType::UInt64,
            "The mapping's unique id within the profile (verbatim); referenced by \
             pprof.locations.mapping_id.",
        ),
        commented(
            "memory_start",
            DataType::UInt64,
            "Start of the address range the binary/library is loaded at.",
        ),
        commented(
            "memory_limit",
            DataType::UInt64,
            "Exclusive end of the loaded address range.",
        ),
        commented(
            "file_offset",
            DataType::UInt64,
            "Offset in the object file that corresponds to memory_start.",
        ),
        commented(
            "filename",
            DataType::Utf8,
            "Object the mapping is loaded from (a path, or a pseudo-name like '[vdso]'); NULL if \
             unset.",
        ),
        commented(
            "build_id",
            DataType::Utf8,
            "Build id (e.g. GNU .note.gnu.build-id) uniquely identifying the binary version, \
             emitted verbatim so unresolved native frames can be symbolized downstream \
             (vgi-symbols); NULL if unset.",
        ),
    ])
}

impl TableFunction for Mappings {
    fn name(&self) -> &str {
        "mappings"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Mappings Table",
            "Decode a pprof profile's mapping table: one row per loaded binary/library with \
             `mapping_id` (verbatim), the loaded address range (`memory_start`, `memory_limit`), \
             `file_offset`, `filename`, and `build_id`. The `build_id` is emitted verbatim so the \
             addresses of unsymbolized native frames flow to a symbolizer (vgi-symbols) for \
             addr→symbol resolution. `src` may be a path, glob, list, or `BLOB`; a bad file yields \
             one error row.",
            "pprof mappings: `mapping_id`, `memory_start`, `memory_limit`, `file_offset`, \
             `filename`, `build_id`. `build_id` feeds a downstream symbolizer.",
            "pprof, mappings, mapping table, build id, build_id, memory range, file offset, \
             binary, library, vdso, symbolization, vgi-symbols",
            "Raw profile graph",
        );
        let mut cols = vec![
            (
                "mapping_id",
                "UBIGINT",
                "The mapping's unique id within the profile (verbatim); referenced by \
                 pprof.locations.mapping_id.",
            ),
            (
                "memory_start",
                "UBIGINT",
                "Start of the address range the binary/library is loaded at.",
            ),
            (
                "memory_limit",
                "UBIGINT",
                "Exclusive end of the loaded address range.",
            ),
            (
                "file_offset",
                "UBIGINT",
                "Offset in the object file that corresponds to memory_start.",
            ),
            (
                "filename",
                "VARCHAR",
                "Object the mapping is loaded from (a path, or a pseudo-name like '[vdso]'); NULL \
                 if unset.",
            ),
            (
                "build_id",
                "VARCHAR",
                "Build id uniquely identifying the binary version, emitted verbatim so unresolved \
                 native frames can be symbolized downstream (vgi-symbols); NULL if unset.",
            ),
        ];
        cols.extend(crate::meta::trailing_result_columns());
        tags.push((
            "vgi.result_columns_schema".into(),
            crate::meta::result_columns_schema(&cols),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        let (examples, example_queries) = crate::meta::described_examples(vec![(
            "List the mappings that carry a build_id (the binaries still to symbolize), from a \
             native profile passed inline as BLOB bytes."
                .into(),
            format!(
                "SELECT mapping_id, filename, build_id \
                 FROM pprof.main.mappings(from_base64('{}')) \
                 WHERE error IS NULL AND build_id IS NOT NULL;",
                crate::meta::NATIVE_B64
            ),
        )]);
        tags.push(("vgi.example_queries".into(), example_queries));
        FunctionMetadata {
            description: "Decode a pprof profile's mapping table (build_id feeds vgi-symbols)"
                .into(),
            examples,
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![source::src_arg_spec()]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: output_schema(),
            opaque_data: Vec::new(),
        })
    }

    fn producer(&self, params: &ProcessParams) -> Result<Box<dyn TableProducer>> {
        let units = source::resolve(&params.arguments, 0)?;
        Ok(Box::new(OneShot::new(
            params.output_schema.clone(),
            units,
            build,
        )))
    }
}

fn build(units: &[DecodeUnit], schema: &SchemaRef) -> Result<RecordBatch> {
    let mut mapping_id: Vec<Option<u64>> = Vec::new();
    let mut memory_start: Vec<Option<u64>> = Vec::new();
    let mut memory_limit: Vec<Option<u64>> = Vec::new();
    let mut file_offset: Vec<Option<u64>> = Vec::new();
    let mut filename: Vec<Option<String>> = Vec::new();
    let mut build_id: Vec<Option<String>> = Vec::new();
    let mut file: Vec<Option<String>> = Vec::new();
    let mut error: Vec<Option<String>> = Vec::new();

    for unit in units {
        match &unit.result {
            Ok(decoded) => {
                for m in decoded.mappings() {
                    mapping_id.push(Some(m.mapping_id));
                    memory_start.push(Some(m.memory_start));
                    memory_limit.push(Some(m.memory_limit));
                    file_offset.push(Some(m.file_offset));
                    filename.push(m.filename);
                    build_id.push(m.build_id);
                    file.push(unit.file.clone());
                    error.push(None);
                }
            }
            Err(msg) => {
                mapping_id.push(None);
                memory_start.push(None);
                memory_limit.push(None);
                file_offset.push(None);
                filename.push(None);
                build_id.push(None);
                file.push(unit.file.clone());
                error.push(Some(msg.clone()));
            }
        }
    }

    RecordBatch::try_new(
        schema.clone(),
        vec![
            ab::u64_col(&mapping_id),
            ab::u64_col(&memory_start),
            ab::u64_col(&memory_limit),
            ab::u64_col(&file_offset),
            ab::str_col(&filename),
            ab::str_col(&build_id),
            ab::str_col(&file),
            ab::str_col(&error),
        ],
    )
    .map_err(|e| RpcError::runtime_error(e.to_string()))
}
