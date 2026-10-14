"""Keep every design run on disk.

The FreeCAD box shows the log and also appends it here, one file per run.
Files are never replaced, so a closed window still has the record.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path


def log_directory(home: Path | None = None) -> Path:
    """Return the directory that holds every saved design log."""
    root = (home or Path.home()) / ".local" / "share" / "kala" / "logs"
    root.mkdir(parents=True, mode=0o700, exist_ok=True)
    return root


@dataclass
class RunLog:
    """One design run, appended as each line arrives."""

    path: Path

    @classmethod
    def start(cls, prompt: str, home: Path | None = None) -> RunLog:
        """Open a new log file and record when the run started."""
        directory = log_directory(home)
        stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        path = directory / f"{stamp}.log"
        suffix = 2
        while path.exists():
            path = directory / f"{stamp}-{suffix}.log"
            suffix += 1
        run = cls(path)
        started = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        run.write(f"# {started}")
        run.write(f"prompt: {prompt.strip()}")
        return run

    def write(self, line: str) -> None:
        """Append one log line and flush it."""
        text = line.replace("\r\n", "\n").replace("\r", "\n")
        with self.path.open("a", encoding="utf-8") as handle:
            handle.write(text)
            if not text.endswith("\n"):
                handle.write("\n")
            handle.flush()
