"""Every design run is kept as its own log file."""

from __future__ import annotations

from pathlib import Path

from kala_agent_freecad.cad.journal import RunLog, log_directory


def test_a_run_is_appended_and_kept(tmp_path: Path) -> None:
    first = RunLog.start("a screw jack", home=tmp_path)
    first.write("Reasoning openrouter/free: a jack")
    first.write("Analysis: off. It runs only when you turn it on.")
    second = RunLog.start("a plate", home=tmp_path)

    saved = sorted(log_directory(tmp_path).glob("*.log"))
    assert len(saved) == 2
    assert first.path in saved
    assert second.path in saved
    text = first.path.read_text(encoding="utf-8")
    assert text.startswith("# ")
    assert "prompt: a screw jack" in text
    assert "Reasoning openrouter/free: a jack" in text
    assert "Analysis: off" in text
    assert second.path.read_text(encoding="utf-8").count("prompt: a plate") == 1
