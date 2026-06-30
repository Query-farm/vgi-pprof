//! Scalar functions exposed by the pprof worker, registered under `pprof.main`.

mod version;

use vgi::Worker;

/// Register every scalar function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_scalar(version::PprofVersion);
}
