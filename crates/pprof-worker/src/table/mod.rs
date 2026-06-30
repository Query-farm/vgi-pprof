//! The six pprof table functions, registered under `pprof.main`.
//!
//! Each decodes the overloaded `src` argument (path / glob / list / BLOB) via
//! [`crate::source`] and flattens the profile into rows. Every table appends two
//! trailing columns — `file` (the source path, NULL for inline BLOB input) and
//! `error` (NULL on success) — so a multi-file glob carries provenance and so a
//! malformed profile surfaces as one **error row** rather than aborting the scan
//! (per-file error capture).

mod functions;
mod locations;
mod mappings;
mod meta;
mod samples;
mod stacks;

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use vgi::table_function::TableProducer;
use vgi::Worker;
use vgi_rpc::{OutputCollector, Result};

use crate::source::DecodeUnit;

/// Register every pprof table function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_table(stacks::Stacks);
    worker.register_table(samples::Samples);
    worker.register_table(functions::Functions);
    worker.register_table(locations::Locations);
    worker.register_table(mappings::Mappings);
    worker.register_table(meta::MetaTable);
}

/// A nullable field carrying a `comment` (surfaced via `duckdb_columns().comment`
/// and counted by the vgi-lint column-comment ratio).
pub fn commented(name: &str, ty: DataType, comment: &str) -> Field {
    Field::new(name, ty, true).with_metadata(HashMap::from([(
        "comment".to_string(),
        comment.to_string(),
    )]))
}

/// The two trailing provenance/error columns shared by every table.
pub fn trailing_fields() -> [Field; 2] {
    [
        commented(
            "file",
            DataType::Utf8,
            "Source path the row came from (NULL when the profile was passed as inline BLOB bytes). \
             Lets a multi-file glob attribute each row to its file.",
        ),
        commented(
            "error",
            DataType::Utf8,
            "NULL on success; otherwise this row is an error row for a file that could not be read \
             or decoded, with every data column NULL and the failure message here.",
        ),
    ]
}

/// Build a [`SchemaRef`] from data fields plus the trailing `file`/`error` pair.
pub fn schema_with_trailing(mut fields: Vec<Field>) -> SchemaRef {
    fields.extend(trailing_fields());
    Arc::new(Schema::new(fields))
}

/// A producer that emits the whole result as a single batch. pprof calls are
/// typically one file per invocation (the headline `glob(...) , pprof.stacks`
/// pattern), so one batch per call keeps the code simple and correct; the build
/// closure turns the decoded units into that batch.
pub struct OneShot {
    schema: SchemaRef,
    units: Option<Vec<DecodeUnit>>,
    build: fn(&[DecodeUnit], &SchemaRef) -> Result<RecordBatch>,
}

impl OneShot {
    pub fn new(
        schema: SchemaRef,
        units: Vec<DecodeUnit>,
        build: fn(&[DecodeUnit], &SchemaRef) -> Result<RecordBatch>,
    ) -> Self {
        OneShot {
            schema,
            units: Some(units),
            build,
        }
    }
}

impl TableProducer for OneShot {
    fn next_batch(&mut self, _out: &mut OutputCollector) -> Result<Option<RecordBatch>> {
        match self.units.take() {
            Some(units) => Ok(Some((self.build)(&units, &self.schema)?)),
            None => Ok(None),
        }
    }
}
