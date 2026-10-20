#!/usr/bin/env python3
"""Probe CPython `re` behaviour that the Rust pattern engine must reproduce."""

from __future__ import annotations

import re
import sys

CASES = [
    r"a{1234567890}",
    r"a{123456789}",
    r"a{4294967295}",
    r"a{4294967296}",
    r"a{2,1}",
    r"a{2",
    r"a{}",
    r"a{,3}",
    r"a{2,}",
    r"a**",
    r"a{2}{3}",
    r"a{2}?",
    r"a{2}??",
    r"a*?",
    r"(?:)",
    r"()",
    r"a|",
    r"|a",
    r"[]]",
    r"[^]]",
    r"[-a]",
    r"[a-]",
    r"[a-\d]",
    r"[\d-x]",
    r"[\b]",
    r"[\w]",
    r"[[:alpha:]]",
    r"\ ",
    r"\#",
    r"\é",
    r"\N{LATIN SMALL LETTER A}",
    r"\N{NOT A REAL NAME}",
    r"\x41",
    r"\x4",
    r"\u0041",
    r"\u041",
    r"\U00000041",
    r"\U0000004",
    r"\0",
    r"\012",
    r"\08",
    r"\777",
    r"\400",
    r"\8",
    r"\q",
    r"\-",
    r"\A\Z",
    r"a$",
    r"\Z",
    r"(?P<n>a)",
    r"(?P<>a)",
    r"(?P<n>a)(?P<n>b)",
    r"(?P<a b>c)",
    r"(?P<n_1>a)",
    r"(?P<é>a)",
    r"(?#comment)",
    r"(?i)x",
    r"a{3,2}",
    r"x{0}",
    r"x{0,0}",
]

SAMPLES = ["", "a", "aa", "aaa", "a\n", "\n", "0123456789", "]", "-", "a\nb", "é", " "]

for case in CASES:
    try:
        compiled = re.compile(case)
    except re.error as error:
        print(f"{case!r}\tERROR\t{error}")
        continue
    except Exception as error:  # noqa: BLE001 - probing non-re.error failures
        print(f"{case!r}\t{type(error).__name__}\t{error}")
        continue
    results = []
    for sample in SAMPLES:
        match = compiled.search(sample)
        results.append("None" if match is None else f"{match.start()}:{match.end()}")
    print(f"{case!r}\tOK\t{results}")
