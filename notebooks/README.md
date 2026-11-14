# Compare profiles on a declared basis

[01 · Compare profiles on a declared basis](01_compare_profiles_on_a_declared_basis.ipynb)
decodes two authored pprof captures, selects CPU values from each file's metadata,
and compares leaf attribution on a declared denominator. A full outer difference
retains categories observed on only one side. The notebook also compares metric
conventions with file metadata and reconstructs leaf names from file-scoped IDs.
[Open in Colab](https://colab.research.google.com/github/Query-farm/vgi-pprof/blob/main/notebooks/01_compare_profiles_on_a_declared_basis.ipynb).

Choose **Runtime → Run all** in Colab. The base shared package preserves compatible
preloaded kernel/data/plotting libraries and retrieves SHA256-verified immutable
companions. A separate local lock includes the `[runner]` extra:

```sh
uv sync --locked
uv run --locked vgi-notebooks
uv run --locked vgi-notebooks 01_compare_profiles_on_a_declared_basis.ipynb --parameters '{"PER_OPERATION":false}' --output-dir artifacts/capture-totals
```

Run from `notebooks/` alone; no worker checkout is needed. For Jupyter, select
`.venv/bin/python`. The runner uses a temporary kernel, preserves successful defaults
on failure and rejects unknown parameters or overrides without a separate output
destination. If an older bootstrap replaced kernel libraries, save your work and
open the current notebook in a fresh runtime.

Visible SQL starts the public native release:

```sql
INSTALL vgi FROM community;
LOAD vgi;
ATTACH 'pprof' (TYPE vgi,LOCATION 'github-auto://Query-farm/vgi-pprof@v0.4.0');
```

Haybarn/VGI downloads the native platform artifact, verifies its SHA256 and caches
it. Initial extension, package, binary and companion setup needs network access.
There is no Rust compiler, worker build, local source checkout, profiling daemon,
credential or external dataset requirement. The local environment locks Haybarn
**2.0.0rc11**, tested on macOS arm64. Upgrade the literal release tag only after
rechecking its catalog, default, experiment and invariants.

The [shared v0.3.15 standard](https://github.com/Query-farm/vgi-example-notebooks/blob/v0.3.15/docs/standard.md) supplies the catalog title/subtitle, branding, isolated SQL
secrets, companion verification, SVG output and execution protections. Keep
`charts/` beside saved notebooks for GitHub's content-addressed SVG image links;
interactive executions retain native SVG. Review PNGs, experiment copies and logs
remain ignored. Hosted Colab was not manually operated; validation uses a fresh
compatible kernel with data/plotting libraries already loaded.

Coverage is **7 of 7 public named SQL surfaces**: `stacks`, `samples`, `functions`,
`locations`, `mappings`, `meta`, and the `sample_type_guide` view. Actual catalogs
contain six table functions and one view. Path/glob/list/BLOB input forms belong
to the same named function; no objects are excluded. `coverage.json` binds actual
released/current runtime exports, complete output columns, execution receipts and
reviewed narrative/SQL/result mappings. The notebook runs the release; compatible
current mappings do not claim current-main notebook execution. Current source
adds metadata and composed examples while decoder/dependency trees and output
schemas stay unchanged.

The three committed profiles are deterministic synthetic data. `build_fixtures.py`
uses Python's standard library to encode the subset of vendored `profile.proto`
needed here; it is not a general-purpose profile writer. Each valid capture has
four leaf-first samples, overlapping file-local IDs, a different authored build ID
and one unsymbolized leaf. No real application was profiled. The authored clock is
**2023-11-14 22:13:20 UTC**, not a collection timestamp. The provenance manifest
records source revision, operation counts, sizes and SHA256 values; execution
verifies all three hashes before decoding.

| Capture | Duration | Authored operations | parse CPU | serialize CPU | legacy CPU | new_cache CPU | Unknown CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 1 s | 100 | 70 ms | 30 ms | 20 ms | 0 ms | 10 ms |
| candidate | 2 s | 200 | 90 ms | 70 ms | 0 ms | 20 ms | 10 ms |

The baseline stores `samples/count` then `cpu/nanoseconds`; the candidate reverses
them. The `sample_type_guide` view explains a common Go CPU convention, not these
files' producer identities. Joining by type/unit shows two of four positions
match that convention. The actual CPU index always comes from each file's metadata.
CPU values already represent nanoseconds and are not multiplied by sampling period.
Sample counts are not completed operations; those are separate authored inputs
using the same synthetic operation definition on both sides.

`broken.pb.gz` is deliberately truncated. Its metadata, stack and function scans
return diagnostic rows with unknown data fields. It has no workload denominator
and never becomes a third zero-CPU cohort. Optional regeneration is local:

```sh
uv run --locked python build_fixtures.py /tmp/vgi-pprof-fixtures
```

Normal execution reads the committed bytes. Previous publication verified all
three regenerated images byte-for-byte; this update preserves their hashes.

Captured CPU is **130 ms baseline / 190 ms candidate**. Dividing by 100/200 completed
operations gives **1.30 / 0.95 ms per operation**. `PER_OPERATION=True` (default)
therefore gives a **−0.35 ms/operation** difference, while the separate **False**
experiment gives **+60 ms total**. The changed denominator changes the question.
Neither result establishes a real performance change; real comparisons need
comparable workloads, capture settings, environments and repeated measurements.

Each sample is attributed once to its leaf. Summing every expanded frame would
double attribution in these two-frame fixtures; inclusive call-tree totals are
not an additive partition. Known leaf keys use function name and source filename.
The unknown category is an aggregate bucket, not matching symbol identities across
builds. Full outer comparison preserves baseline-only `legacy` and candidate-only
`new_cache`. Zero filling means no observed leaf sample in a successfully decoded
capture, never missing/failed input or proof of absence from an executable.

Raw graph joins use **file plus original profile ID**. The `functions` projection
reconstructs six named leaves and preserves two address-only leaves, agreeing with
flattened stacks. Both `legacy` and `new_cache` are defined in both function tables,
even where their leaf count is zero. These locations contain at most one line entry;
general inline profiles require the complete ordered line list. Build IDs and
binary offsets remain available for later symbolization, but no matching executable
is supplied and no symbol names are invented.

Eighteen SQL relationship checks plus chart reconciliation cover metric selection,
error retention, graph identities, NULL leaves and the selected difference.
Validation also includes defaults, a separate notebook-only parameter experiment,
two reviewed SVG figures, three runner guards and one fresh preloaded bootstrap
for this companion/launcher path. Existing green
[source CI at 863cddd](https://github.com/Query-farm/vgi-pprof/actions/runs/43577950119)
is reused; no worker rebuild, full-suite repetition or new worker CI is triggered.
Prior validation passed 16 Rust tests including a doctest, 102 stdio / 116 HTTP /
102 Unix SQL assertions, Clippy, rustfmt, warning-clean rustdoc and metadata
100/100 L2 with zero findings. Worker source, original data and Cargo files are unchanged.
