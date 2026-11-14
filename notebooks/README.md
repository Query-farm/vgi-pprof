# VGI pprof notebooks

[01 · Compare profiles on a declared basis](01_compare_profiles_on_a_declared_basis.ipynb) uses visible SQL to decode synthetic pprof captures, select CPU values from metadata, compare leaf attribution, and retain both sides of a full outer difference. The experiment switches between per-operation values and capture totals.

## Run

Install [uv](https://docs.astral.sh/uv/). From the repository root:

```sh
uv sync --project notebooks --frozen
uv run --project notebooks --frozen python notebooks/execute.py
```

The complete `notebooks/` directory also runs independently of the worker checkout:

```sh
cd notebooks
uv sync --frozen
uv run --frozen python execute.py
```

Initial setup downloads the Python environment, community VGI extension and pinned native worker release. Analysis then reads only committed synthetic profile bytes. No Rust toolchain, profiling daemon, local worker build, credentials or external dataset is required. Execution was verified on macOS Apple silicon. The release also publishes native Linux x86-64/arm64 and Windows x86-64 archives.

In VS Code select `notebooks/.venv/bin/python` and use `notebooks/` as the kernel working directory. For another Jupyter editor, optionally register it:

```sh
uv run --project notebooks --frozen python -m ipykernel install --user --name vgi-pprof-notebooks --display-name 'Python (vgi-pprof notebooks)'
```

Restart and run all after editing parameters. VGI connection and analytical SQL remain expanded; imports, presentation, plotting, assertions and cleanup source are folded. Saved outputs embed both charts.

## Experiment

```sh
uv run --project notebooks --frozen python notebooks/execute.py \
  --parameters '{"PER_OPERATION":false}' \
  --output-dir notebooks/validation/capture-totals
```

The default is `PER_OPERATION=True`. Setting it to false changes the question and chart units, while preserving raw CPU values, workload metadata and decoding results. Reset it to true for the saved default.

The runner requires a separate destination for overrides, rejects unknown names and saves only successful executions. A failed run preserves saved notebook outputs. Experiment copies include branding, fixtures, helpers, environment and documentation. Review charts, logs and experimental copies are ignored by Git.

## Portable startup and fixture provenance

The literal SQL location is `github-auto://Query-farm/vgi-pprof@v0.4.0`; VGI downloads, verifies and caches the platform archive. The isolated lock pins Haybarn 2.0.0rc11. Upgrade by changing the release tag explicitly and rerunning both settings.

`build_fixtures.py` uses Python's standard library to encode the small subset of the repository's vendored `profile.proto` used by these authored examples. It writes deterministic gzip-wrapped protobuf with four leaf-first samples per capture, overlapping file-local IDs, different build IDs and one unsymbolized leaf per capture. It is a fixture encoder, not a general-purpose profile writer. No real application was profiled.

| Capture | Duration | Authored operations | parse CPU | serialize CPU | legacy CPU | new_cache CPU | Unknown CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 1 s | 100 | 70 ms | 30 ms | 20 ms | 0 ms | 10 ms |
| candidate | 2 s | 200 | 90 ms | 70 ms | 0 ms | 20 ms | 10 ms |

The baseline stores `samples/count` then `cpu/nanoseconds`; the candidate reverses those slots. The CPU value index is selected from each file's metadata, not hardcoded. These values already represent CPU nanoseconds and are not multiplied by sampling period. Sample counts are not completed operations. Operations are separate authored manifest inputs with the same synthetic operation definition on both sides.

`broken.pb.gz` is intentionally truncated. Its metadata and stack scans return an error row; it is a diagnostic fixture with no workload count, not a third comparison cohort. Failed input is never zero-filled into the comparison.

Optional regeneration:

```sh
uv run --project notebooks --frozen python notebooks/build_fixtures.py /tmp/vgi-pprof-fixtures
```

All three regenerated files were checked byte-for-byte. Normal notebook execution only reads the committed files.

## Interpretation

Captured CPU totals are 130 ms and 190 ms. With 100 and 200 completed operations, the corresponding values are 1.30 and 0.95 ms per operation. Changing the basis reverses the capture-level difference from −0.35 ms per operation to +60 ms total. Neither result establishes a real performance change: these are deliberately authored arithmetic examples. Real experiments need comparable workload composition, capture settings, environments and repeated measurements.

Each sample is attributed once to its leaf. Expanding and summing every frame would double attribution in these two-frame fixtures; inclusive call-tree totals are not an additive partition. Known leaf keys use function name and source filename. The unsymbolized category remains an aggregate unknown bucket, not a claim that unknown functions match across builds.

A full outer join retains categories observed in only one capture. Zero filling means no observed leaf sample in a successfully decoded capture, not absence from the executable. `legacy` remains visible from the baseline; `new_cache` remains visible from the candidate. Raw graph joins use both the file and the original profile ID, preserving build ID and binary offset for a later symbolization step. There is no corresponding executable in this fixture, so the notebook does not invent symbol names or invoke a symbolization worker.

Validation covers default and detached experiment executions, four chart renderings, twelve SQL invariants plus chart reconciliation, three runner guards, and fixture regeneration. Source validation passed 16 Rust tests including a doctest, 102 stdio / 116 HTTP / 102 Unix SQL assertions across seven cases per transport, Clippy, rustfmt, warning-clean rustdoc and the locked worker build. Metadata scored 100/100 L2 with zero findings. Source, tests, original data and Cargo files are preserved. The default run took 8.0 seconds; the detached totals experiment took 1.9 seconds. Profile metadata, raw CPU sample and address-handoff displays were identical between settings.

Official Query.Farm Strata Sun and VGI artwork is copied unchanged from the approved assets, with Query.Farm branding at the top and bottom.
