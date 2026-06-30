// Copyright 2026 Query Farm LLC - https://query.farm
//
// Generate the golden pprof fixtures used by the pprof-core tests and the
// haybarn SQLLogic E2E. We build them with the canonical google/pprof/profile
// package — the same library `go tool pprof` uses — so the bytes are authentic
// pprof (gzip-wrapped profile.proto), one fixture per producer shape:
//
//   go_cpu.pb.gz    Go CPU profile         sample_type [samples/count, cpu/nanoseconds]
//   go_cpu2.pb.gz   Go CPU profile (v2)    same shape, shifted values (regression diff)
//   go_heap.pb.gz   Go heap profile        4 value types (alloc/inuse objects+space)
//   alloc.pb.gz     multi-value alloc      [alloc_objects/count, alloc_space/bytes]
//   native.pb.gz    unsymbolized native    mappings carry build_id, locations are addr-only
//
// Run from this directory:  go run generate_fixtures.go
package main

import (
	"log"
	"os"

	"github.com/google/pprof/profile"
)

func write(name string, p *profile.Profile) {
	if err := p.CheckValid(); err != nil {
		log.Fatalf("%s: invalid profile: %v", name, err)
	}
	f, err := os.Create(name)
	if err != nil {
		log.Fatal(err)
	}
	defer f.Close()
	if err := p.Write(f); err != nil {
		log.Fatalf("%s: %v", name, err)
	}
	log.Printf("wrote %s", name)
}

// fn builds a Function with a single source file.
func fn(id uint64, name, file string, start int64) *profile.Function {
	return &profile.Function{ID: id, Name: name, SystemName: name, Filename: file, StartLine: start}
}

// loc builds a symbolized Location: one or more inlined lines (innermost first).
func loc(id uint64, m *profile.Mapping, addr uint64, lines ...profile.Line) *profile.Location {
	return &profile.Location{ID: id, Mapping: m, Address: addr, Line: lines}
}

func goCPU(scale int64) *profile.Profile {
	st := []*profile.ValueType{
		{Type: "samples", Unit: "count"},
		{Type: "cpu", Unit: "nanoseconds"},
	}
	m := &profile.Mapping{ID: 1, Start: 0x400000, Limit: 0x500000, File: "/usr/local/bin/app", HasFunctions: true}
	fMain := fn(1, "main.main", "main.go", 10)
	fWork := fn(2, "main.work", "work.go", 20)
	fHot := fn(3, "main.hotLoop", "work.go", 40)
	fParse := fn(4, "main.parse", "parse.go", 5)

	lMain := loc(1, m, 0x401000, profile.Line{Function: fMain, Line: 12})
	lWork := loc(2, m, 0x401100, profile.Line{Function: fWork, Line: 22})
	lHot := loc(3, m, 0x401200, profile.Line{Function: fHot, Line: 42})
	lParse := loc(4, m, 0x401300, profile.Line{Function: fParse, Line: 7})

	p := &profile.Profile{
		SampleType:        st,
		DefaultSampleType: "cpu",
		PeriodType:        &profile.ValueType{Type: "cpu", Unit: "nanoseconds"},
		Period:            10000000,
		DurationNanos:     1000000000,
		TimeNanos:         1700000000000000000,
		Mapping:           []*profile.Mapping{m},
		Function:          []*profile.Function{fMain, fWork, fHot, fParse},
		Location:          []*profile.Location{lMain, lWork, lHot, lParse},
		Sample: []*profile.Sample{
			// hotLoop <- work <- main : the busy path
			{Location: []*profile.Location{lHot, lWork, lMain}, Value: []int64{30, 30 * scale * 1000000}},
			// parse <- main : a lighter path
			{Location: []*profile.Location{lParse, lMain}, Value: []int64{5, 5 * 1000000}},
		},
	}
	return p
}

func goHeap() *profile.Profile {
	st := []*profile.ValueType{
		{Type: "alloc_objects", Unit: "count"},
		{Type: "alloc_space", Unit: "bytes"},
		{Type: "inuse_objects", Unit: "count"},
		{Type: "inuse_space", Unit: "bytes"},
	}
	m := &profile.Mapping{ID: 1, Start: 0x400000, Limit: 0x500000, File: "/usr/local/bin/app", HasFunctions: true}
	fMain := fn(1, "main.main", "main.go", 10)
	fAlloc := fn(2, "main.allocate", "alloc.go", 30)
	// One location with two inlined frames (allocate inlined into a helper).
	fHelper := fn(3, "main.makeBuf", "alloc.go", 50)
	lMain := loc(1, m, 0x401000, profile.Line{Function: fMain, Line: 12})
	lAlloc := loc(2, m, 0x401100,
		profile.Line{Function: fHelper, Line: 52}, // innermost (inlined)
		profile.Line{Function: fAlloc, Line: 33},  // caller
	)
	p := &profile.Profile{
		SampleType:        st,
		DefaultSampleType: "inuse_space",
		PeriodType:        &profile.ValueType{Type: "space", Unit: "bytes"},
		Period:            524288,
		TimeNanos:         1700000000000000000,
		Mapping:           []*profile.Mapping{m},
		Function:          []*profile.Function{fMain, fAlloc, fHelper},
		Location:          []*profile.Location{lMain, lAlloc},
		Sample: []*profile.Sample{
			{Location: []*profile.Location{lAlloc, lMain}, Value: []int64{100, 1048576, 40, 409600}},
		},
	}
	return p
}

func allocMultiValue() *profile.Profile {
	st := []*profile.ValueType{
		{Type: "alloc_objects", Unit: "count"},
		{Type: "alloc_space", Unit: "bytes"},
	}
	m := &profile.Mapping{ID: 1, Start: 0x400000, Limit: 0x500000, File: "/srv/server", HasFunctions: true}
	fServe := fn(1, "server.serve", "server.go", 100)
	fBuf := fn(2, "server.newBuffer", "buffer.go", 8)
	lServe := loc(1, m, 0x402000, profile.Line{Function: fServe, Line: 110})
	lBuf := loc(2, m, 0x402100, profile.Line{Function: fBuf, Line: 9})
	p := &profile.Profile{
		SampleType:        st,
		DefaultSampleType: "alloc_space",
		PeriodType:        &profile.ValueType{Type: "space", Unit: "bytes"},
		Period:            524288,
		TimeNanos:         1700000000000000000,
		Mapping:           []*profile.Mapping{m},
		Function:          []*profile.Function{fServe, fBuf},
		Location:          []*profile.Location{lServe, lBuf},
		Sample: []*profile.Sample{
			{
				Location: []*profile.Location{lBuf, lServe},
				Value:    []int64{2048, 4194304},
				Label:    map[string][]string{"size_class": {"4096"}},
			},
		},
	}
	return p
}

func nativeUnsymbolized() *profile.Profile {
	st := []*profile.ValueType{
		{Type: "samples", Unit: "count"},
		{Type: "cpu", Unit: "nanoseconds"},
	}
	// Two mappings, each with a build_id but NO symbol info: locations are
	// address-only (no Line/Function), exactly what an unsymbolized native
	// profile looks like before symbolication.
	exe := &profile.Mapping{ID: 1, Start: 0x55000000, Limit: 0x55010000, BuildID: "a1b2c3d4e5f6", File: "/opt/svc/bin/svc"}
	lib := &profile.Mapping{ID: 2, Start: 0x7f0000000000, Limit: 0x7f0000100000, BuildID: "deadbeefcafe", File: "/lib/x86_64-linux-gnu/libc.so.6"}
	l1 := &profile.Location{ID: 1, Mapping: exe, Address: 0x55000420}
	l2 := &profile.Location{ID: 2, Mapping: lib, Address: 0x7f0000001234}
	l3 := &profile.Location{ID: 3, Mapping: exe, Address: 0x550008ab}
	p := &profile.Profile{
		SampleType:    st,
		PeriodType:    &profile.ValueType{Type: "cpu", Unit: "nanoseconds"},
		Period:        10000000,
		DurationNanos: 500000000,
		TimeNanos:     1700000000000000000,
		Mapping:       []*profile.Mapping{exe, lib},
		Location:      []*profile.Location{l1, l2, l3},
		Sample: []*profile.Sample{
			{Location: []*profile.Location{l1, l2}, Value: []int64{12, 120000000}},
			{Location: []*profile.Location{l3, l1}, Value: []int64{7, 70000000}},
		},
	}
	return p
}

func main() {
	write("go_cpu.pb.gz", goCPU(1))
	write("go_cpu2.pb.gz", goCPU(3)) // candidate build: hotLoop got 3x heavier
	write("go_heap.pb.gz", goHeap())
	write("alloc.pb.gz", allocMultiValue())
	write("native.pb.gz", nativeUnsymbolized())
}
