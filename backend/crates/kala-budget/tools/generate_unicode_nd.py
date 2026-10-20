"""Generate the compact Unicode Nd digit table used by `src/unicode.rs`.

CPython's `decimal.Decimal()` and `float()` accept every character with the
Unicode `Nd` (decimal digit) property and translate it to the matching ASCII
digit. Rust's `char::is_ascii_digit`/`to_digit` cover only ASCII, so the crate
embeds a compact table of contiguous Nd runs. Inside a run the digit value is
`(codepoint - start) % 10` (runs may contain several 0..9 cycles, as with the
mathematical digit blocks).

Provenance: this script was run once with the repository virtual environment
Python 3.14.8, whose `unicodedata.unidata_version` is 16.0.0. Regenerate with:

    .venv/bin/python backend/crates/kala-budget/tools/generate_unicode_nd.py

The generated table is checked in; Python is never required at build or run
time (it is only a development/test oracle).
"""

from __future__ import annotations

import sys
import unicodedata


def nd_runs() -> list[tuple[int, int]]:
    runs: list[tuple[int, int]] = []
    start: int | None = None
    previous: int | None = None
    for codepoint in range(sys.maxunicode + 1):
        if unicodedata.decimal(chr(codepoint), None) is not None:
            if start is None:
                start = previous = codepoint
            elif codepoint == previous + 1:
                previous = codepoint
            else:
                runs.append((start, previous))
                start = previous = codepoint
    if start is not None and previous is not None:
        runs.append((start, previous))
    return runs


def main() -> None:
    runs = nd_runs()
    for start, end in runs:
        for codepoint in range(start, end + 1):
            expected = (codepoint - start) % 10
            actual = unicodedata.decimal(chr(codepoint))
            if actual != expected:
                raise SystemExit(
                    f"run {start:#x}..{end:#x} is not a cyclic 0..9 sequence: "
                    f"{codepoint:#x} has value {actual}, expected {expected}"
                )
    print(f"// Unicode {unicodedata.unidata_version}: {len(runs)} runs")
    print("pub(super) const ND_DIGIT_BLOCKS: &[(u32, u32)] = &[")
    for start, end in runs:
        print(f"    (0x{start:04X}, 0x{end:04X}),")
    print("];")


if __name__ == "__main__":
    main()
