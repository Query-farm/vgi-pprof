//! Golden tests over real pprof fixtures produced by `data/generate_fixtures.go`
//! (built with google/pprof — the same library `go tool pprof` uses), one per
//! producer shape, plus property-based fuzzing asserting the decoder never
//! panics on truncated / oversized / arbitrary input.

use pprof_core::Decoded;
use proptest::prelude::*;

/// Read a fixture from the repo `data/` directory (relative to this crate).
fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../../data/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn decode(name: &str) -> Decoded {
    Decoded::from_bytes(&fixture(name)).unwrap_or_else(|e| panic!("decode {name}: {e}"))
}

#[test]
fn go_cpu_shapes() {
    let d = decode("go_cpu.pb.gz");
    let m = d.meta();
    assert_eq!(m.sample_types.len(), 2);
    assert_eq!(m.sample_types[0].r#type, "samples");
    assert_eq!(m.sample_types[0].unit, "count");
    assert_eq!(m.sample_types[1].r#type, "cpu");
    assert_eq!(m.sample_types[1].unit, "nanoseconds");
    assert_eq!(m.default_sample_type.as_deref(), Some("cpu"));
    assert_eq!(m.period, 10_000_000);
    assert_eq!(m.period_type.unwrap().r#type, "cpu");

    let stacks = d.stacks();
    assert_eq!(stacks.len(), 2);
    // The busy path is hotLoop <- work <- main (leaf first).
    let busy = stacks.iter().max_by_key(|s| s.value[1]).expect("a sample");
    assert_eq!(busy.frames.len(), 3);
    assert_eq!(busy.frames[0].function.as_deref(), Some("main.hotLoop"));
    assert_eq!(busy.frames[1].function.as_deref(), Some("main.work"));
    assert_eq!(busy.frames[2].function.as_deref(), Some("main.main"));
    assert_eq!(busy.frames[0].filename.as_deref(), Some("work.go"));
    assert_eq!(busy.value[1], 30_000_000);

    // value[1] is cpu nanoseconds; both samples are aligned to the 2 sample types.
    assert!(stacks.iter().all(|s| s.value.len() == 2));

    let funcs = d.functions();
    assert_eq!(funcs.len(), 4);
    assert!(funcs
        .iter()
        .any(|f| f.name.as_deref() == Some("main.parse")));

    let maps = d.mappings();
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].filename.as_deref(), Some("/usr/local/bin/app"));
    assert!(maps[0].build_id.is_none());
}

#[test]
fn go_heap_is_multi_value_with_inlined_frames() {
    let d = decode("go_heap.pb.gz");
    let m = d.meta();
    assert_eq!(m.sample_types.len(), 4);
    let types: Vec<&str> = m.sample_types.iter().map(|t| t.r#type.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "alloc_objects",
            "alloc_space",
            "inuse_objects",
            "inuse_space"
        ]
    );
    assert_eq!(m.default_sample_type.as_deref(), Some("inuse_space"));

    let stacks = d.stacks();
    assert_eq!(stacks.len(), 1);
    let s = &stacks[0];
    assert_eq!(s.value, vec![100, 1_048_576, 40, 409_600]);
    // The allocating location carries two inlined frames (makeBuf inlined into
    // allocate), then main → 3 frames, innermost first.
    assert_eq!(s.frames.len(), 3);
    assert_eq!(s.frames[0].function.as_deref(), Some("main.makeBuf"));
    assert_eq!(s.frames[1].function.as_deref(), Some("main.allocate"));
    assert_eq!(s.frames[2].function.as_deref(), Some("main.main"));
}

#[test]
fn alloc_multi_value_with_label() {
    let d = decode("alloc.pb.gz");
    let m = d.meta();
    assert_eq!(m.sample_types.len(), 2);
    assert_eq!(m.sample_types[1].r#type, "alloc_space");

    let stacks = d.stacks();
    assert_eq!(stacks.len(), 1);
    assert_eq!(stacks[0].value, vec![2048, 4_194_304]);
    assert_eq!(
        stacks[0].labels,
        vec![("size_class".to_string(), "4096".to_string())]
    );
}

#[test]
fn native_is_unsymbolized_with_build_ids() {
    let d = decode("native.pb.gz");
    let maps = d.mappings();
    assert_eq!(maps.len(), 2);
    let build_ids: Vec<&str> = maps.iter().filter_map(|m| m.build_id.as_deref()).collect();
    assert!(build_ids.contains(&"a1b2c3d4e5f6"));
    assert!(build_ids.contains(&"deadbeefcafe"));

    // Locations are address-only (no line table).
    let locs = d.locations();
    assert_eq!(locs.len(), 3);
    assert!(locs.iter().all(|l| l.lines.is_empty()));
    assert!(locs.iter().all(|l| l.address != 0));

    // Stacks have address-only frames: function/filename NULL, address present.
    let stacks = d.stacks();
    assert_eq!(stacks.len(), 2);
    for s in &stacks {
        for fr in &s.frames {
            assert!(fr.function.is_none(), "native frames are unsymbolized");
            assert!(fr.address != 0, "native frames keep their address");
        }
    }
}

#[test]
fn two_profile_regression_diff() {
    // The same query the haybarn E2E runs: hotLoop's cpu_ns grows 3x between the
    // baseline (go_cpu) and candidate (go_cpu2) builds.
    let leaf_ns = |name: &str| -> i64 {
        decode(name)
            .stacks()
            .into_iter()
            .filter(|s| {
                s.frames.first().and_then(|f| f.function.as_deref()) == Some("main.hotLoop")
            })
            .map(|s| s.value[1])
            .sum()
    };
    let base = leaf_ns("go_cpu.pb.gz");
    let cand = leaf_ns("go_cpu2.pb.gz");
    assert_eq!(base, 30_000_000);
    assert_eq!(cand, 90_000_000);
    assert_eq!(cand - base, 60_000_000);
}

// ---- property-based fuzzing: the decoder never panics --------------------

/// Drive every flattener so a successful-but-garbage decode can't panic later.
fn exercise(d: &Decoded) {
    let _ = d.meta();
    let _ = d.stacks();
    let _ = d.samples();
    let _ = d.functions();
    let _ = d.locations();
    let _ = d.mappings();
}

proptest! {
    // Arbitrary bytes: decode must return Ok or Err, never panic. If it decodes
    // (a chance proto), the flatteners must also not panic.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(d) = Decoded::from_bytes(&bytes) {
            exercise(&d);
        }
    }

    // Truncated real profiles: every prefix of a real fixture must not panic.
    #[test]
    fn truncated_fixtures_never_panic(
        which in 0usize..5,
        cut in 0usize..400,
    ) {
        let names = ["go_cpu.pb.gz", "go_heap.pb.gz", "alloc.pb.gz", "native.pb.gz", "go_cpu2.pb.gz"];
        let mut bytes = fixture(names[which]);
        let n = cut.min(bytes.len());
        bytes.truncate(n);
        if let Ok(d) = Decoded::from_bytes(&bytes) {
            exercise(&d);
        }
    }

    // Oversized / corrupted-tail input: a valid profile with random trailing
    // bytes, plus a large random blob, must not panic.
    #[test]
    fn oversized_and_tailed_never_panic(
        tail in proptest::collection::vec(any::<u8>(), 0..8192),
    ) {
        let mut bytes = fixture("go_cpu.pb.gz");
        bytes.extend_from_slice(&tail);
        if let Ok(d) = Decoded::from_bytes(&bytes) {
            exercise(&d);
        }
        // A big non-gzip blob exercises the raw-proto decode path under size.
        let big = vec![0x08u8; 200_000];
        let _ = Decoded::from_bytes(&big).map(|d| exercise(&d));
    }
}
