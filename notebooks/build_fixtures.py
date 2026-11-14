"""Write small deterministic synthetic pprof fixtures using only Python's stdlib.

This encodes the minimal fields used by these authored fixtures, following the
repository's vendored profile.proto. It is not a general-purpose profile writer.
Run with an output directory; the notebook reads committed bytes instead.
"""

import argparse
import gzip
from pathlib import Path


def varint(value: int) -> bytes:
    """Encode a nonnegative protobuf integer."""
    output = bytearray()
    while value > 127:
        output.append((value & 127) | 128)
        value >>= 7
    output.append(value)
    return bytes(output)


def integer(field: int, value: int) -> bytes:
    """Encode a varint field."""
    return varint(field << 3) + varint(value)


def message(field: int, value: bytes) -> bytes:
    """Encode a length-delimited field."""
    return varint((field << 3) | 2) + varint(len(value)) + value


def profile(candidate: bool) -> bytes:
    """Encode four leaf-first samples with declared CPU/count slots."""
    strings = [
        "",
        "samples",
        "count",
        "cpu",
        "nanoseconds",
        "main.run",
        "parse",
        "serialize",
        "legacy",
        "new_cache",
        "synthetic.go",
        "/synthetic/app",
        "candidate-build" if candidate else "baseline-build",
    ]
    index = {value: i for i, value in enumerate(strings)}

    def value_type(name: str, unit: str) -> bytes:
        return integer(1, index[name]) + integer(2, index[unit])

    kinds = (
        [("cpu", "nanoseconds"), ("samples", "count")]
        if candidate
        else [("samples", "count"), ("cpu", "nanoseconds")]
    )
    output = b"".join(message(1, value_type(*kind)) for kind in kinds)
    values = (
        [(2, 90), (3, 70), (5, 20), (6, 10)] if candidate else [(2, 70), (3, 30), (4, 20), (6, 10)]
    )
    for location, cpu_ms in values:
        slots = [cpu_ms * 1_000_000, cpu_ms] if candidate else [cpu_ms, cpu_ms * 1_000_000]
        sample = message(1, varint(location) + varint(1))
        sample += message(2, b"".join(varint(v) for v in slots))
        output += message(2, sample)
    mapping = integer(1, 1) + integer(2, 0x400000) + integer(3, 0x500000)
    mapping += integer(5, index["/synthetic/app"]) + integer(6, index[strings[-1]])
    output += message(3, mapping)
    for function_id, name in enumerate(
        ["main.run", "parse", "serialize", "legacy", "new_cache"], 1
    ):
        function = integer(1, function_id) + integer(2, index[name]) + integer(3, index[name])
        function += integer(4, index["synthetic.go"]) + integer(5, function_id * 10)
        output += message(5, function)
        location = integer(1, function_id) + integer(2, 1) + integer(3, 0x400000 + function_id * 16)
        location += message(4, integer(1, function_id) + integer(2, function_id * 10))
        output += message(4, location)
    # Address-only leaf: preserve unknown symbol identity rather than invent one.
    output += message(4, integer(1, 6) + integer(2, 1) + integer(3, 0x400060))
    output += b"".join(message(6, s.encode()) for s in strings)
    output += integer(9, 1_700_000_000_000_000_000)
    output += integer(10, 2_000_000_000 if candidate else 1_000_000_000)
    output += message(11, value_type("cpu", "nanoseconds"))
    output += integer(12, 1_000_000) + integer(14, index["cpu"])
    return gzip.compress(output, mtime=0)


def main() -> None:
    """Generate the two profiles and an intentionally truncated gzip file."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for name, candidate in [("baseline", False), ("candidate", True)]:
        (args.output / f"{name}.pb.gz").write_bytes(profile(candidate))
    (args.output / "broken.pb.gz").write_bytes(b"\x1f\x8b\x08\x00truncated")


if __name__ == "__main__":
    main()
