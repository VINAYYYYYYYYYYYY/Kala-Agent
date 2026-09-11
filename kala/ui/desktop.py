"""Kala — CAD agent console (companion to FreeCAD, not a chatbot landing page)."""

from __future__ import annotations

import json
import sys
import traceback
from pathlib import Path

from PySide6.QtCore import QPoint, Qt, QThread, Signal
from PySide6.QtGui import QColor, QFont, QFontDatabase, QKeyEvent, QPalette
from PySide6.QtWidgets import (
    QApplication,
    QFrame,
    QHBoxLayout,
    QLabel,
    QMainWindow,
    QMessageBox,
    QMenu,
    QPushButton,
    QPlainTextEdit,
    QScrollArea,
    QVBoxLayout,
    QWidget,
)

from kala.ui.providers_dialog import ProvidersDialog

from kala.procedures import list_procedures, load_default_procedure, suggest_procedure

# (label, prompt, procedure_id) — procedure ids from packaged library
STARTERS: list[tuple[str, str, str]] = [
    (
        "L-bracket",
        "L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step",
        "simple_bracket",
    ),
    (
        "Flanged bracket",
        "Flanged mounting bracket: vertical plate 120×80×10 mm fused to base flange "
        "160×100×12 mm, central bore Ø40 mm, four M10 clearance holes Ø11 mm inset 15 mm. Export STEP.",
        "simple_bracket",
    ),
    (
        "Shaft bushing",
        "Shaft bushing OD Ø34 mm, bore Ø28 mm, length 40 mm. Export STEP.",
        "stepped_shaft",
    ),
    (
        "Plate holes",
        "Rectangular plate 100x60x8 mm with four Ø6 holes inset 10 mm from corners. Export STEP.",
        "plate_with_holes",
    ),
    (
        "Housing cover",
        "Housing cover plate 120x80x6 mm with central Ø40 bore and four M6 clearance holes. Export STEP.",
        "housing_cover",
    ),
]

# Smoke: library must load (also documents available playbooks for the rail).
_LIBRARY_IDS = {p.id for p in list_procedures()}

APP_QSS = """
* {
  font-family: "iA Writer Quattro S", "Adwaita Sans", "Noto Sans", sans-serif;
}
QMainWindow, QWidget {
  background-color: #0e1011;
  color: #c4c9cf;
  font-size: 13px;
}
QScrollArea, QScrollArea > QWidget > QWidget {
  background-color: #0e1011;
  border: none;
}
QLabel#brand {
  color: #f0f1f2;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.1em;
}
QLabel#dim, QLabel#railDim {
  color: #5c636c;
  font-size: 11px;
  font-family: "iA Writer Mono S", "Adwaita Mono", monospace;
}
QLabel#railTitle {
  color: #aeb4bc;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.14em;
}
QLabel#railValue {
  color: #e8eaed;
  font-size: 12px;
  font-family: "iA Writer Mono S", "Adwaita Mono", monospace;
}
QLabel#empty {
  color: #5c636c;
  font-size: 13px;
}
QWidget#rail {
  background-color: #101214;
  border: none;
}
QLabel#you, QLabel#kala {
  color: #5c636c;
  font-size: 10px;
  letter-spacing: 0.1em;
  font-weight: 600;
}
QLabel#body {
  color: #d8dce1;
  font-size: 13px;
}
QLabel#toolName {
  font-family: "iA Writer Mono S", "JetBrainsMono Nerd Font", "Adwaita Mono", monospace;
  color: #e4e7eb;
  font-size: 12px;
}
QLabel#toolMeta {
  font-family: "iA Writer Mono S", "JetBrainsMono Nerd Font", "Adwaita Mono", monospace;
  color: #5c636c;
  font-size: 11px;
}
QLabel#ok {
  color: #6f8f78;
  font-family: "iA Writer Mono S", monospace;
  font-size: 11px;
}
QLabel#fail {
  color: #b86e6e;
  font-family: "iA Writer Mono S", monospace;
  font-size: 11px;
}
QLabel#proc {
  color: #6b7280;
}
QLabel#procOn {
  color: #a8c4b0;
}
QLabel#procDone {
  color: #5c636c;
}
QFrame#hairline {
  background-color: #1c2024;
  max-height: 1px;
  min-height: 1px;
  border: none;
}
QFrame#vline {
  background-color: #1c2024;
  max-width: 1px;
  min-width: 1px;
  border: none;
}
QFrame#composerShell {
  background-color: #141719;
  border: 1px solid #252a30;
  border-radius: 2px;
}
QFrame#toolOk {
  background: transparent;
  border-left: 2px solid #3d5a48;
}
QFrame#toolFail {
  background: transparent;
  border-left: 2px solid #6e3d3d;
}
QFrame#userMsg {
  background: transparent;
  border-left: 2px solid #2a3038;
}
QWidget#rail {
  background-color: #101214;
}
QLabel#proc {
  color: #6b7280;
  font-size: 11px;
  font-family: "iA Writer Mono S", monospace;
}
QLabel#procOn {
  color: #a8c4b0;
  font-size: 11px;
  font-family: "iA Writer Mono S", monospace;
  font-weight: 600;
}
QLabel#procDone {
  color: #5c636c;
  font-size: 11px;
  font-family: "iA Writer Mono S", monospace;
}
QPushButton#link {
  background: transparent;
  color: #7a828c;
  border: none;
  padding: 2px 0;
  text-align: left;
  font-size: 12px;
}
QPushButton#link:hover { color: #d0d4d9; }
QPushButton#icon {
  background: transparent;
  color: #6b7280;
  border: 1px solid transparent;
  border-radius: 2px;
  padding: 4px 8px;
  font-size: 12px;
  font-weight: 500;
}
QPushButton#icon:hover {
  color: #d0d4d9;
  border-color: #2a3038;
  background: #141719;
}
QPushButton#run {
  background-color: #24352c;
  color: #dce6df;
  border: 1px solid #355043;
  border-radius: 2px;
  padding: 6px 14px;
  font-weight: 600;
  font-size: 12px;
  letter-spacing: 0.04em;
  min-width: 64px;
}
QPushButton#run:hover { background-color: #2d4236; }
QPushButton#run:disabled {
  background-color: #171a1d;
  color: #3e444c;
  border-color: #252a30;
}
QPlainTextEdit#composer {
  background: transparent;
  color: #e4e7eb;
  border: none;
  padding: 2px;
  font-size: 13.5px;
  selection-background-color: #355043;
}
QMenu {
  background-color: #141719;
  color: #c4c9cf;
  border: 1px solid #252a30;
  padding: 4px;
}
QMenu::item { padding: 6px 16px; }
QMenu::item:selected { background-color: #24352c; }
"""


class RunWorker(QThread):
    """Run the agent in a *subprocess* so FreeCAD's Part.so is never loaded
    into the Qt UI process (that mix SIGSEGVs on import during heavy runs)."""

    finished_ok = Signal(dict)
    failed = Signal(str)

    def __init__(self, goal: str, backend: str, standard_parts: bool, procedure: str) -> None:
        super().__init__()
        self.goal = goal
        self.backend = backend
        self.standard_parts = standard_parts
        self.procedure = procedure

    def run(self) -> None:
        import os
        import subprocess
        from pathlib import Path

        repo = Path(__file__).resolve().parents[2]
        out_json = repo / "outputs" / "logs" / "_ui_run.json"
        out_json.parent.mkdir(parents=True, exist_ok=True)
        parts = "on" if self.standard_parts else "off"
        kala_bin = Path(sys.executable).resolve().parent / "kala"
        cmd = [
            str(kala_bin if kala_bin.is_file() else sys.executable),
            *(
                []
                if kala_bin.is_file()
                else ["-m", "kala.cli"]
            ),
            "run",
            self.goal,
            "--backend",
            self.backend,
            "--standard-parts",
            parts,
            "--procedure",
            self.procedure,
            "--json",
        ]
        try:
            proc = subprocess.run(
                cmd,
                cwd=str(repo),
                capture_output=True,
                text=True,
                timeout=900,
                env={**os.environ, "PYTHONUNBUFFERED": "1"},
                check=False,
            )
        except subprocess.TimeoutExpired:
            self.failed.emit("Run timed out after 15 minutes")
            return
        except Exception:  # noqa: BLE001
            self.failed.emit(traceback.format_exc())
            return

        raw = (proc.stdout or "").strip()
        if proc.returncode != 0 and not raw:
            err = (proc.stderr or "").strip() or f"exit {proc.returncode}"
            self.failed.emit(err[-2000:])
            return
        # --json prints a single JSON object
        try:
            # Prefer last JSON object if any logging leaked
            start = raw.find("{")
            if start < 0:
                raise ValueError(raw[-500:] or proc.stderr or "empty output")
            payload = json.loads(raw[start:])
        except Exception as exc:  # noqa: BLE001
            self.failed.emit(
                f"Could not parse agent output: {exc}\n"
                f"stderr: {(proc.stderr or '')[-800:]}\n"
                f"stdout: {raw[-800:]}"
            )
            return
        try:
            out_json.write_text(json.dumps(payload), encoding="utf-8")
        except OSError:
            pass
        self.finished_ok.emit(payload)

class Composer(QPlainTextEdit):
    submit = Signal()

    def keyPressEvent(self, event: QKeyEvent) -> None:  # noqa: N802
        if event.key() in (Qt.Key.Key_Return, Qt.Key.Key_Enter):
            if event.modifiers() & Qt.KeyboardModifier.ShiftModifier:
                super().keyPressEvent(event)
            else:
                self.submit.emit()
            return
        super().keyPressEvent(event)


class ToolLine(QFrame):
    def __init__(self, event: dict) -> None:
        super().__init__()
        ok = bool(event.get("ok"))
        self.setObjectName("toolOk" if ok else "toolFail")
        lay = QVBoxLayout(self)
        lay.setContentsMargins(10, 3, 4, 3)
        lay.setSpacing(1)

        row = QHBoxLayout()
        row.setSpacing(8)
        status = QLabel("ok" if ok else "fail")
        status.setObjectName("ok" if ok else "fail")
        name = QLabel(str(event.get("tool") or "—"))
        name.setObjectName("toolName")
        row.addWidget(status)
        row.addWidget(name, 1)
        lay.addLayout(row)

        args = event.get("args") or {}
        if args:
            s = json.dumps(args, ensure_ascii=False)
            if len(s) > 100:
                s = s[:97] + "…"
            meta = QLabel(s)
            meta.setObjectName("toolMeta")
            meta.setWordWrap(True)
            lay.addWidget(meta)

        cad = (event.get("data") or {}).get("cad_api")
        if cad:
            c = QLabel(str(cad))
            c.setObjectName("toolMeta")
            c.setWordWrap(True)
            lay.addWidget(c)

        if not ok:
            msg = (event.get("message") or "").splitlines()[0][:140]
            if msg:
                e = QLabel(msg)
                e.setObjectName("fail")
                e.setWordWrap(True)
                lay.addWidget(e)


def _row(key: str, value: str) -> tuple[QWidget, QLabel]:
    """Dense property-browser row: KEY  value."""
    box = QWidget()
    lay = QHBoxLayout(box)
    lay.setContentsMargins(0, 0, 0, 0)
    lay.setSpacing(8)
    k = QLabel(key)
    k.setObjectName("railTitle")
    k.setFixedWidth(72)
    v = QLabel(value)
    v.setObjectName("railValue")
    v.setWordWrap(True)
    v.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
    lay.addWidget(k, 0, Qt.AlignmentFlag.AlignTop)
    lay.addWidget(v, 1)
    return box, v


class MainWindow(QMainWindow):
    def __init__(self, default_backend: str = "freecad") -> None:
        super().__init__()
        self.setWindowTitle("Kala")
        self.resize(980, 860)
        self.setMinimumSize(720, 560)
        self._force_opaque()

        self._worker: RunWorker | None = None
        self._proc: dict[str, QLabel] = {}
        self._proc_box: QVBoxLayout | None = None
        self._thinking: QWidget | None = None
        self._empty = True
        self._default_backend = default_backend
        self._backend_id = default_backend
        self._parts_on = False
        self._procedure_id = "simple_bracket"

        shell = QWidget()
        shell.setAutoFillBackground(True)
        self.setCentralWidget(shell)
        outer = QVBoxLayout(shell)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        outer.addWidget(self._top())
        line = QFrame()
        line.setObjectName("hairline")
        outer.addWidget(line)

        body = QHBoxLayout()
        body.setContentsMargins(0, 0, 0, 0)
        body.setSpacing(0)
        body.addWidget(self._thread_pane(), 1)
        vline = QFrame()
        vline.setObjectName("vline")
        body.addWidget(vline)
        body.addWidget(self._rail())
        outer.addLayout(body, 1)

        line2 = QFrame()
        line2.setObjectName("hairline")
        outer.addWidget(line2)
        outer.addWidget(self._composer_block())

        self._paint_empty()
        self._refresh_planner()
        self._mark_proc(None, done=False)
        self._rail_set("idle", export="—", tools="—", sync="—")
        self._hydrate_last_run()

    def _hydrate_last_run(self) -> None:
        """Show last session so the console never opens as a blank landing page."""
        try:
            from kala.session.runlog import agent_latest_path

            path = agent_latest_path()
            if not path.is_file():
                return
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001
            return

        state = payload.get("state") or {}
        goal = str(payload.get("goal") or state.get("goal") or "").strip()
        history = list(state.get("history") or [])
        tools = [e for e in history if e.get("tool") != "show_in_freecad"]
        if not goal and not tools:
            return

        self._wipe_feed()
        self._empty = False
        self.starters.setVisible(True)
        if goal:
            self._add(self._user_msg(goal))
        # Keep the feed readable — last handful of tool calls
        shown = tools[-10:] if len(tools) > 10 else tools
        status = state.get("status") or "done"
        n = len(tools)
        fails = sum(1 for e in tools if not e.get("ok"))
        summary = f"last run · {status} · {n} tools"
        if fails:
            summary += f" · {fails} failed"
        if len(tools) > 10:
            summary += f" · showing last {len(shown)}"
        export = state.get("last_export") or ""
        if export:
            summary += f"\n{Path(export).name}"
        self._add(self._agent_msg(summary, tools=shown))

        step = (state.get("procedure") or {}).get("step")
        if step is None and status == "done":
            self._mark_proc(None, done=True)
        elif step:
            self._mark_proc(str(step.get("id")), done=False)

        self._rail_set(
            str(status),
            export=Path(export).name if export else "—",
            tools=f"{n}" + (f" · {fails} fail" if fails else ""),
            sync="—",
        )

    def _force_opaque(self) -> None:
        pal = self.palette()
        bg = QColor("#0e1011")
        pal.setColor(QPalette.ColorRole.Window, bg)
        pal.setColor(QPalette.ColorRole.Base, bg)
        self.setPalette(pal)
        self.setAutoFillBackground(True)

    def _top(self) -> QWidget:
        bar = QWidget()
        bar.setFixedHeight(38)
        lay = QHBoxLayout(bar)
        lay.setContentsMargins(16, 0, 12, 0)
        lay.setSpacing(10)

        brand = QLabel("KALA")
        brand.setObjectName("brand")
        lay.addWidget(brand)
        lay.addStretch(1)

        self.planner_lbl = QLabel("")
        self.planner_lbl.setObjectName("dim")
        lay.addWidget(self.planner_lbl)

        self.status = QLabel("")
        self.status.setObjectName("dim")
        lay.addWidget(self.status)

        fc = QPushButton("FreeCAD")
        fc.setObjectName("icon")
        fc.setCursor(Qt.CursorShape.PointingHandCursor)
        fc.clicked.connect(self._open_fc)
        lay.addWidget(fc)

        more = QPushButton("···")
        more.setObjectName("icon")
        more.setCursor(Qt.CursorShape.PointingHandCursor)
        more.clicked.connect(lambda: self._menu(more))
        lay.addWidget(more)
        return bar

    def _thread_pane(self) -> QWidget:
        pane = QWidget()
        lay = QVBoxLayout(pane)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(0)

        self.scroll = QScrollArea()
        self.scroll.setWidgetResizable(True)
        self.scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.thread = QWidget()
        self.feed = QVBoxLayout(self.thread)
        self.feed.setContentsMargins(20, 20, 16, 16)
        self.feed.setSpacing(14)
        self.feed.addStretch(1)
        self.scroll.setWidget(self.thread)
        lay.addWidget(self.scroll, 1)
        return pane

    def _rail(self) -> QWidget:
        rail = QWidget()
        rail.setObjectName("rail")
        rail.setFixedWidth(248)
        outer = QVBoxLayout(rail)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        inner = QWidget()
        lay = QVBoxLayout(inner)
        lay.setContentsMargins(12, 14, 12, 14)
        lay.setSpacing(8)

        head = QLabel("SESSION")
        head.setObjectName("railTitle")
        lay.addWidget(head)

        box, self.rail_status = _row("STATUS", "idle")
        lay.addWidget(box)
        box, self.rail_backend = _row("BACKEND", self._backend_id)
        lay.addWidget(box)
        box, self.rail_sync = _row("FREECAD", "—")
        lay.addWidget(box)
        box, self.rail_tools = _row("TOOLS", "—")
        lay.addWidget(box)
        box, self.rail_export = _row("EXPORT", "—")
        lay.addWidget(box)

        sep = QFrame()
        sep.setObjectName("hairline")
        lay.addWidget(sep)

        proc_h = QLabel("PROCEDURE")
        proc_h.setObjectName("railTitle")
        lay.addWidget(proc_h)
        self.rail_proc_id = QLabel(self._procedure_id)
        self.rail_proc_id.setObjectName("railDim")
        lay.addWidget(self.rail_proc_id)

        self._proc_box = QVBoxLayout()
        self._proc_box.setContentsMargins(0, 0, 0, 0)
        self._proc_box.setSpacing(2)
        lay.addLayout(self._proc_box)
        self._rebuild_proc_rail(self._procedure_id)

        lay.addStretch(1)

        tip = QLabel("Geometry in FreeCAD.\nKala plans the tools.")
        tip.setObjectName("railDim")
        tip.setWordWrap(True)
        lay.addWidget(tip)

        scroll.setWidget(inner)
        outer.addWidget(scroll)
        return rail

    def _rail_set(self, status: str, *, export: str, tools: str, sync: str) -> None:
        self.rail_status.setText(status)
        self.rail_backend.setText(self._backend_id)
        self.rail_export.setText(export)
        self.rail_tools.setText(tools)
        self.rail_sync.setText(sync)

    def _menu(self, anchor: QWidget) -> None:
        menu = QMenu(self)
        menu.addAction("OpenRouter API…").triggered.connect(self._open_api)
        menu.addSeparator()

        be_fc = menu.addAction("Backend: FreeCAD")
        be_fc.setCheckable(True)
        be_mock = menu.addAction("Backend: Mock")
        be_mock.setCheckable(True)
        be_fc.setChecked(self._backend_id == "freecad")
        be_mock.setChecked(self._backend_id == "mock")
        be_fc.triggered.connect(lambda: self._set_backend("freecad"))
        be_mock.triggered.connect(lambda: self._set_backend("mock"))

        menu.addSeparator()
        parts = menu.addAction("Standard parts")
        parts.setCheckable(True)
        parts.setChecked(self._parts_on)
        parts.triggered.connect(lambda checked: setattr(self, "_parts_on", checked))
        menu.exec(anchor.mapToGlobal(QPoint(0, anchor.height() + 4)))

    def _set_backend(self, name: str) -> None:
        self._backend_id = name
        self.rail_backend.setText(name)

    def _composer_block(self) -> QWidget:
        wrap = QWidget()
        lay = QVBoxLayout(wrap)
        lay.setContentsMargins(16, 10, 16, 14)
        lay.setSpacing(8)

        self.starters = QWidget()
        srow = QHBoxLayout(self.starters)
        srow.setContentsMargins(0, 0, 0, 0)
        srow.setSpacing(16)
        for label, prompt, proc_id in STARTERS:
            btn = QPushButton(label)
            btn.setObjectName("link")
            btn.setCursor(Qt.CursorShape.PointingHandCursor)
            btn.clicked.connect(
                lambda _=False, p=prompt, pid=proc_id: self._fill_starter(p, pid)
            )
            srow.addWidget(btn)
        srow.addStretch(1)
        lay.addWidget(self.starters)

        shell = QFrame()
        shell.setObjectName("composerShell")
        slay = QVBoxLayout(shell)
        slay.setContentsMargins(12, 10, 12, 10)
        slay.setSpacing(8)

        self.composer = Composer()
        self.composer.setObjectName("composer")
        self.composer.setPlaceholderText("Part brief in mm — Enter runs · Shift+Enter newline")
        self.composer.setFixedHeight(68)
        self.composer.submit.connect(self._send)
        slay.addWidget(self.composer)

        row = QHBoxLayout()
        hint = QLabel("mm · STEP export · playbook gated")
        hint.setObjectName("dim")
        row.addWidget(hint)
        row.addStretch(1)
        self.run_btn = QPushButton("Run")
        self.run_btn.setObjectName("run")
        self.run_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        self.run_btn.clicked.connect(self._send)
        row.addWidget(self.run_btn)
        slay.addLayout(row)
        lay.addWidget(shell)
        return wrap

    def _wipe_feed(self) -> None:
        while self.feed.count():
            item = self.feed.takeAt(0)
            w = item.widget()
            if w is not None:
                w.deleteLater()

    def _paint_empty(self) -> None:
        self._wipe_feed()
        self._empty = True
        self.starters.setVisible(True)
        hint = QLabel("Tool calls show up here after you run a brief.")
        hint.setObjectName("empty")
        hint.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        self.feed.addWidget(hint)
        self.feed.addStretch(1)

    def _fill(self, text: str) -> None:
        self.composer.setPlainText(text)
        self.composer.setFocus()

    def _fill_starter(self, text: str, procedure_id: str) -> None:
        self._set_procedure(procedure_id)
        self._fill(text)

    def _set_procedure(self, procedure_id: str) -> None:
        self._procedure_id = procedure_id
        self._rebuild_proc_rail(procedure_id)

    def _rebuild_proc_rail(self, procedure_id: str) -> None:
        """Rebuild procedure rail labels from packaged library step ids."""
        if self._proc_box is None:
            return
        while self._proc_box.count():
            item = self._proc_box.takeAt(0)
            w = item.widget()
            if w is not None:
                w.deleteLater()
        self._proc.clear()
        try:
            proc = load_default_procedure(procedure_id)
            steps = [s.id for s in proc.steps]
        except Exception:  # noqa: BLE001
            steps = ["envelope", "features", "standard_parts", "export"]
        if hasattr(self, "rail_proc_id"):
            self.rail_proc_id.setText(procedure_id)
        for step in steps:
            lbl = QLabel(f"  {step}")
            lbl.setObjectName("proc")
            self._proc[step] = lbl
            self._proc_box.addWidget(lbl)
        self._mark_proc(None, done=False)

    def _drop(self, w: QWidget | None) -> None:
        if w is None:
            return
        self.feed.removeWidget(w)
        w.deleteLater()

    def _add(self, w: QWidget) -> None:
        if self.feed.count():
            last = self.feed.itemAt(self.feed.count() - 1)
            if last and last.spacerItem():
                self.feed.takeAt(self.feed.count() - 1)
        self.feed.addWidget(w)
        self.feed.addStretch(1)
        QApplication.processEvents()
        sb = self.scroll.verticalScrollBar()
        sb.setValue(sb.maximum())

    def _user_msg(self, text: str) -> QWidget:
        box = QFrame()
        box.setObjectName("userMsg")
        lay = QVBoxLayout(box)
        lay.setContentsMargins(12, 2, 4, 2)
        lay.setSpacing(4)
        who = QLabel("YOU")
        who.setObjectName("you")
        body = QLabel(text)
        body.setObjectName("body")
        body.setWordWrap(True)
        body.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        lay.addWidget(who)
        lay.addWidget(body)
        return box

    def _agent_msg(self, text: str, tools: list[dict] | None = None) -> QWidget:
        box = QWidget()
        lay = QVBoxLayout(box)
        lay.setContentsMargins(2, 2, 2, 2)
        lay.setSpacing(6)
        who = QLabel("KALA")
        who.setObjectName("kala")
        body = QLabel(text)
        body.setObjectName("body")
        body.setWordWrap(True)
        body.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        lay.addWidget(who)
        lay.addWidget(body)
        if tools:
            for ev in tools:
                if ev.get("tool") == "show_in_freecad":
                    continue
                lay.addWidget(ToolLine(ev))
        return box

    def _mark_proc(self, current: str | None, *, done: bool) -> None:
        seen = False
        for step, lbl in self._proc.items():
            if done:
                lbl.setObjectName("procDone")
                lbl.setText(f"✓ {step}")
            elif current is None:
                lbl.setObjectName("proc")
                lbl.setText(f"  {step}")
            elif step == current:
                lbl.setObjectName("procOn")
                lbl.setText(f"▸ {step}")
                seen = True
            elif not seen:
                lbl.setObjectName("procDone")
                lbl.setText(f"✓ {step}")
            else:
                lbl.setObjectName("proc")
                lbl.setText(f"  {step}")
            lbl.style().unpolish(lbl)
            lbl.style().polish(lbl)

    def _refresh_planner(self) -> None:
        from kala.llm.providers import looks_like_api_key, load_store

        a = load_store().active()
        if a is None or not looks_like_api_key(a.api_key, a.provider):
            self.planner_lbl.setText("no API key")
            return
        m = a.model
        self.planner_lbl.setText(m if len(m) <= 36 else m[:33] + "…")

    def _open_api(self) -> None:
        if ProvidersDialog(self).exec():
            self._refresh_planner()

    def _open_fc(self) -> None:
        try:
            from kala.cad.freecad.gui_sync import ensure_live_shown, mod_heartbeat_fresh

            note = ensure_live_shown()
            tag = "live" if mod_heartbeat_fresh(8.0) else "opened"
            self.rail_sync.setText(tag)
            if "failed" in note.lower() or "missing" in note.lower()[:20]:
                QMessageBox.warning(self, "FreeCAD", note)
        except Exception as exc:  # noqa: BLE001
            QMessageBox.warning(self, "FreeCAD", str(exc))

    def _set_busy(self, busy: bool) -> None:
        self.run_btn.setEnabled(not busy)
        self.status.setText("working…" if busy else "")
        if busy:
            self._rail_set("running", export=self.rail_export.text(), tools="…", sync=self.rail_sync.text())

    def _send(self) -> None:
        from kala.llm.providers import looks_like_api_key, load_store

        goal = self.composer.toPlainText().strip()
        if not goal:
            return
        if self._worker and self._worker.isRunning():
            return

        a = load_store().active()
        if a is None or not looks_like_api_key(a.api_key, a.provider):
            QMessageBox.warning(
                self,
                "Planner needs an API key",
                "Open ··· → OpenRouter API… and paste a key from openrouter.ai/keys (sk-or-…)."
            )
            return

        if self._empty:
            self._wipe_feed()
            self.feed.addStretch(1)
            self._empty = False
        self.starters.setVisible(False)

        self._add(self._user_msg(goal))
        self.composer.clear()
        self._set_busy(True)
        suggested = suggest_procedure(goal)
        if suggested in _LIBRARY_IDS:
            self._set_procedure(suggested)
        first = next(iter(self._proc), "envelope")
        self._mark_proc(first, done=False)

        self._thinking = self._agent_msg("Working…")
        self._add(self._thinking)

        self._worker = RunWorker(
            goal=goal,
            backend=self._backend_id,
            standard_parts=self._parts_on,
            procedure=self._procedure_id,
        )
        self._worker.finished_ok.connect(self._done)
        self._worker.failed.connect(self._fail)
        self._worker.start()

    def _done(self, payload: dict) -> None:
        self._set_busy(False)
        self._drop(self._thinking)
        self._thinking = None

        state = payload.get("state") or {}
        history = list(state.get("history") or [])
        status = state.get("status") or "done"
        step = (state.get("procedure") or {}).get("step")
        gui = state.get("gui_note") or ""
        export = state.get("last_export") or ""

        tools = [e for e in history if e.get("tool") != "show_in_freecad"]
        n = len(tools)
        fails = sum(1 for e in tools if not e.get("ok"))
        lines = [f"{status} · {n} tools"]
        if fails:
            lines[0] += f" · {fails} failed"
        if export:
            lines.append(Path(export).name)
        elif gui:
            lines.append(gui.split(".")[0])

        self._add(self._agent_msg("\n".join(lines), tools=history))

        if step is None and status == "done":
            self._mark_proc(None, done=True)
        elif step:
            self._mark_proc(str(step.get("id")), done=False)

        sync = "—"
        if "Opened FreeCAD" in gui or "force_open" in gui:
            sync = "in FreeCAD"
        elif "heartbeat ok" in gui.lower():
            sync = "synced"

        exp = Path(export).name if export else "—"
        self._rail_set(
            status,
            export=exp,
            tools=f"{n}" + (f" · {fails} fail" if fails else ""),
            sync=sync,
        )
        self.status.setText("" if status == "done" else status)

    def _fail(self, message: str) -> None:
        from kala.ui.error_copy import sanitize_error

        self._set_busy(False)
        self._drop(self._thinking)
        self._thinking = None
        friendly = sanitize_error(message)
        self._add(self._agent_msg(friendly))
        self.status.setText("error")
        self._rail_set("error", export=self.rail_export.text(), tools="—", sync=self.rail_sync.text())


def _apply_fonts(app: QApplication) -> None:
    for family in ("iA Writer Quattro S", "Adwaita Sans", "Noto Sans"):
        if family in QFontDatabase.families():
            app.setFont(QFont(family, 13))
            break


def run_app(default_backend: str = "freecad") -> int:
    app = QApplication.instance() or QApplication(sys.argv)
    app.setApplicationName("Kala")
    app.setOrganizationName("Kala")
    app.setDesktopFileName("kala")
    app.setStyle("Fusion")
    _apply_fonts(app)
    app.setStyleSheet(APP_QSS)
    win = MainWindow(default_backend=default_backend)
    win.show()
    return app.exec()


def main() -> None:
    raise SystemExit(run_app())


if __name__ == "__main__":
    main()
