//! `pprof.mappings(src)` — the mapping table: mapping_id → memory range /
//! file_offset / filename / build_id. `build_id` feeds `vgi-symbols`.

use arrow_array::RecordBatch;
use arrow_schema::{DataType, SchemaRef};
use vgi::table_function::{TableFunction, TableProducer};
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use crate::arrow_build as ab;
use crate::source::{self, DecodeUnit};
use crate::table::{commented, schema_with_trailing, OneShot};

const EXECUTABLE_EXAMPLES: &str = r#"[
  {
    "description": "Mappings with build ids (the addresses that still need symbolizing).",
    "sql": "SELECT mapping_id, filename, build_id FROM pprof.main.mappings('data/native.pb.gz') WHERE error IS NULL AND build_id IS NOT NULL"
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
             addr→symbol resolution. `src` may be a path, glob, list, or BLOB; a bad file yields \
             one error row.",
            "pprof mappings: `mapping_id`, `memory_start`, `memory_limit`, `file_offset`, \
             `filename`, `build_id`. `build_id` feeds a downstream symbolizer.",
            "pprof, mappings, mapping table, build id, build_id, memory range, file offset, \
             binary, library, vdso, symbolization, vgi-symbols",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "| column | type | description |\n\
             |---|---|---|\n\
             | `mapping_id` | UBIGINT | Mapping id (verbatim). |\n\
             | `memory_start` | UBIGINT | Load address range start. |\n\
             | `memory_limit` | UBIGINT | Load address range end. |\n\
             | `file_offset` | UBIGINT | Object-file offset of memory_start. |\n\
             | `filename` | VARCHAR | Object path / pseudo-name. |\n\
             | `build_id` | VARCHAR | Build id (feeds vgi-symbols). |\n\
             | `file` | VARCHAR | Source path (NULL for BLOB input). |\n\
             | `error` | VARCHAR | NULL on success, else the decode error. |"
                .into(),
        ));
        tags.push(("vgi.executable_examples".into(), EXECUTABLE_EXAMPLES.into()));
        FunctionMetadata {
            description: "Decode a pprof profile's mapping table (build_id feeds vgi-symbols)"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT mapping_id, filename, build_id FROM pprof.main.mappings('native.pb.gz') \
                      WHERE error IS NULL AND build_id IS NOT NULL;"
                    .into(),
                description: "List mappings with a build_id (the binaries still to symbolize)."
                    .into(),
                expected_output: None,
            }],
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
