//! Compile the vendored `proto/profile.proto` (Google pprof, Apache-2.0) into
//! Rust with `prost-build`. We point `PROTOC` at the hermetic
//! `protoc-bin-vendored` binary so the build needs no system `protoc` (works
//! identically on every CI runner and developer machine).

use std::path::PathBuf;

fn main() {
    let proto = "proto/profile.proto";
    println!("cargo:rerun-if-changed={proto}");

    // Use the vendored protoc so there is no system dependency. An explicit
    // PROTOC override (e.g. a distro package) still wins if the caller set one.
    if std::env::var_os("PROTOC").is_none() {
        let protoc: PathBuf = protoc_bin_vendored::protoc_bin_path()
            .expect("protoc-bin-vendored: no bundled protoc for this platform");
        std::env::set_var("PROTOC", protoc);
    }

    prost_build::compile_protos(&[proto], &["proto"]).expect("failed to compile profile.proto");
}
