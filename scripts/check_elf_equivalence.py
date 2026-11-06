#!/usr/bin/env python3
"""Read-only comparison of ELF64 runtime sections, allowing build-ID changes."""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def read_elf(path):
    data = path.read_bytes()
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    assert header[0][:6] == b"\x7fELF\x02\x01", "requires little-endian ELF64"
    rows = [struct.unpack_from("<IIQQQQIIQQ", data, header[6] + i * header[11])
            for i in range(header[12])]
    names = rows[header[13]]
    names = data[names[4]:names[4] + names[5]]
    sections = {}
    for row in rows:
        name = names[row[0]:].split(b"\0", 1)[0].decode()
        content = b"" if row[1] == 8 else data[row[4]:row[4] + row[5]]
        assert row[1] == 8 or len(content) == row[5]
        sections[name] = dict(type=row[1], flags=row[2], address=row[3], offset=row[4],
            size=row[5], alignment=row[8], sha256=hashlib.sha256(content).hexdigest())
    assert len(sections) == len(rows), "duplicate section names"
    return data, sections, data[header[5]:header[5] + header[9] * header[10]]


def main(args):
    a, left, ph_a = read_elf(args.baseline)
    b, right, ph_b = read_elf(args.candidate)
    assert a[:64] == b[:64] and ph_a == ph_b, "ELF load layout changed"
    assert left.keys() == right.keys(), "ELF section set changed"
    changed = [name for name in left if left[name] != right[name]]
    runtime = [name for name in changed if left[name]["flags"] & 2]
    assert runtime in ([], [".note.gnu.build-id"]), runtime
    result = dict(measured_binary_sha256=hashlib.sha256(a).hexdigest(),
        final_binary_sha256=hashlib.sha256(b).hexdigest(), changed_sections=changed,
        changed_allocated_sections=runtime, executable_and_runtime_data_sections_identical=True,
        final_sections=right)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "final_sections"}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("baseline", "candidate", "output"):
        parser.add_argument("--" + option, type=Path, required=True)
    main(parser.parse_args())
