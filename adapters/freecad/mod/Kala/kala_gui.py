"""The Kala design box. This is a real module so FreeCAD can keep it loaded."""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import FreeCADGui as Gui
from PySide import QtCore, QtWidgets


def _prepare_path() -> None:
    repo = Path(__file__).resolve().parents[4]
    roots = [repo / "adapters" / "freecad" / "src", repo / "src"]
    venv = repo / ".venv" / "lib"
    if venv.is_dir():
        roots.extend(path for path in venv.glob("python*/site-packages") if path.is_dir())
    for root in roots:
        text = root.as_posix()
        if root.is_dir() and text not in sys.path:
            sys.path.insert(0, text)


def _dock_area() -> QtCore.Qt.DockWidgetArea:
    nested = getattr(QtCore.Qt, "DockWidgetArea", None)
    if nested is not None and hasattr(nested, "RightDockWidgetArea"):
        return nested.RightDockWidgetArea  # type: ignore[no-any-return]
    return QtCore.Qt.RightDockWidgetArea  # type: ignore[attr-defined, no-any-return]


class _Planner(QtCore.QThread):
    log_line = QtCore.Signal(str)
    ready = QtCore.Signal(str)
    failed = QtCore.Signal(str)

    def __init__(self, prompt: str, analyze: bool) -> None:
        super().__init__()
        self._prompt = prompt
        self._analyze = analyze

    def run(self) -> None:
        try:
            from kala_agent_freecad.cad.compose import design_request

            plan = design_request(self._prompt, on_log=self.log_line.emit, analyze=self._analyze)
            payload: dict[str, object] = {
                "analyze": self._analyze,
                "calls": plan.as_json(),
                "prompt": self._prompt,
                "summary": plan.summary,
            }
            script = getattr(plan, "script", None)
            if script is not None:
                payload["script"] = script
            self.ready.emit(json.dumps(payload))
        except Exception as error:
            self.failed.emit(str(error))


class KalaDock(QtWidgets.QDockWidget):
    """Prompt, decision, and the part, all in the FreeCAD window."""

    def __init__(self, parent: QtWidgets.QWidget) -> None:
        super().__init__("Kala", parent)
        self.setObjectName("KalaDesign")
        body = QtWidgets.QWidget(self)
        layout = QtWidgets.QVBoxLayout(body)
        self._prompt = QtWidgets.QPlainTextEdit(body)
        self._prompt.setPlaceholderText("a screw jack, or a 80 x 40 x 8 mm plate")
        self._prompt.setFixedHeight(88)
        _prepare_path()
        from kala_agent_freecad.cad.models import api_key, decide_key

        self._key: QtWidgets.QLineEdit | None = None
        self._decide: QtWidgets.QLineEdit | None = None
        if not api_key():
            self._key = QtWidgets.QLineEdit(body)
            self._key.setPlaceholderText("OpenRouter key for the reasoning model")
            self._key.setEchoMode(QtWidgets.QLineEdit.EchoMode.Password)
        if not decide_key():
            self._decide = QtWidgets.QLineEdit(body)
            self._decide.setPlaceholderText("OpenRouter key for the decision model")
            self._decide.setEchoMode(QtWidgets.QLineEdit.EchoMode.Password)
        self._analyze = QtWidgets.QCheckBox("Run analysis", body)
        self._analyze.setChecked(False)
        self._build = QtWidgets.QPushButton("Build in this window", body)
        self._status = QtWidgets.QLabel(
            "Write a request. The part is built here, not in a browser.", body
        )
        self._status.setWordWrap(True)
        self._log = QtWidgets.QPlainTextEdit(body)
        self._log.setReadOnly(True)
        self._log.setPlaceholderText(
            "Reasoning, each decision, and each FreeCAD tool are written here."
        )
        layout.addWidget(self._prompt)
        if self._key is not None:
            layout.addWidget(self._key)
        if self._decide is not None:
            layout.addWidget(self._decide)
        layout.addWidget(self._analyze)
        layout.addWidget(self._build)
        layout.addWidget(self._status)
        layout.addWidget(self._log)
        self.setWidget(body)
        self._worker: _Planner | None = None
        self._journal: object | None = None
        self._build.clicked.connect(self._start)

    def _start(self) -> None:
        prompt = self._prompt.toPlainText().strip()
        if not prompt:
            self._status.setText("The request is empty.")
            return
        if self._key is not None:
            key = self._key.text().strip()
            if key:
                os.environ["OPENROUTER_API_KEY"] = key
        if self._decide is not None:
            key = self._decide.text().strip()
            if key:
                os.environ["OPENROUTER_DECIDE_API_KEY"] = key
        self._log.clear()
        self._open_journal(prompt)
        waiting, blocked = self._waiting()
        if blocked:
            self._status.setText(waiting)
            return
        self._build.setEnabled(False)
        self._status.setText(waiting)
        worker = _Planner(prompt, self._analyze.isChecked())
        worker.log_line.connect(self._append)
        worker.ready.connect(self._build_calls)
        worker.failed.connect(self._fail)
        self._worker = worker
        worker.start()

    def _waiting(self) -> tuple[str, bool]:
        return "Deciding the parts…", False

    def _open_journal(self, prompt: str) -> None:
        from kala_agent_freecad.cad.journal import RunLog

        try:
            self._journal = RunLog.start(prompt)
        except OSError:
            self._journal = None
            self._log.appendPlainText("The design log could not be saved.")

    def _append(self, line: str) -> None:
        self._log.appendPlainText(line)
        self._log.verticalScrollBar().setValue(self._log.verticalScrollBar().maximum())
        journal = self._journal
        write = getattr(journal, "write", None)
        if not callable(write):
            return
        try:
            write(line)
        except OSError:
            self._journal = None

    def _fail(self, message: str) -> None:
        self._append(message)
        self._status.setText(message)
        self._build.setEnabled(True)

    def _build_calls(self, payload: str) -> None:
        try:
            parsed = json.loads(payload)
            summary = str(parsed.get("summary", ""))
            calls = json.dumps(parsed["calls"])
            self._append("FreeCAD")
            self._status.setText(summary or "Building in this window.")
            measurements = self._run(calls, automatic_redesign=False)
            if parsed.get("analyze") and isinstance(parsed.get("script"), dict):
                self._revise(
                    parsed,
                    {
                        "analysis_detail": measurements.get("analysis_detail", ""),
                        "analysis_status": measurements.get("analysis_status", ""),
                        "peak_stress_mpa": measurements.get("peak_stress_mpa"),
                    },
                )
            self._status.setText(summary or "The model is open in this window.")
        except Exception as error:
            self._append(str(error))
            self._status.setText(str(error))
        finally:
            self._build.setEnabled(True)

    def _revise(self, parsed: dict[str, object], measurements: dict[str, object]) -> None:
        from kala_agent.contracts import ConstructionError, load_construction
        from kala_agent_freecad.cad.compose import revise_design

        prompt = parsed.get("prompt")
        raw_script = parsed.get("script")
        if not isinstance(raw_script, dict):
            return
        try:
            script = load_construction(raw_script)
        except ConstructionError as error:
            self._append(str(error))
            return
        revised = revise_design(
            prompt if isinstance(prompt, str) else "",
            script,
            measurements,
            on_log=self._append,
        )
        if revised is None:
            return
        self._append(revised.summary)
        self._run(json.dumps(revised.as_json()), automatic_redesign=False)

    def _run(self, calls_json: str, *, automatic_redesign: bool = True) -> dict[str, object]:
        import FreeCAD as App
        import ObjectsFem
        import Part

        from kala_agent_freecad.availability import resolve_calculix
        from kala_agent_freecad.cad.design import parse_request, run_open
        from kala_agent_freecad.cad.runtime import FreeCADModules

        try:
            Gui.activateWorkbench("PartDesignWorkbench")
        except Exception:
            pass
        version = App.Version()
        modules = FreeCADModules(
            freecad=App,
            part=Part,
            objects_fem=ObjectsFem,
            ccx_tools=None,
            version=".".join(str(item) for item in list(version)[:3]),
            import_root="",
            import_order=("FreeCAD", "Part", "ObjectsFem"),
            solver_bridge_detail="",
        )
        request = parse_request({"calls_json": calls_json, "document_name": "Kala"})

        def show(tool: str, sketch_name: str) -> None:
            self._append(f"  {tool}" if not sketch_name else f"  {tool} {sketch_name}")
            document = App.ActiveDocument
            if document is not None:
                document.recompute()
            gui = Gui.ActiveDocument
            if gui is None and document is not None:
                Gui.ActiveDocument = Gui.getDocument(document.Name)
                gui = Gui.ActiveDocument
            if gui is not None and tool == "sketch.new" and sketch_name:
                gui.setEdit(sketch_name)
            if gui is not None and tool == "sketch.leave":
                gui.resetEdit()
            view = getattr(gui, "ActiveView", None) if gui is not None else None
            if view is not None:
                if tool in {
                    "partdesign.body",
                    "partdesign.pad",
                    "partdesign.helix",
                    "sketch.leave",
                }:
                    view.viewIsometric()
                view.fitAll()
            Gui.updateGui()
            QtWidgets.QApplication.processEvents()
            time.sleep(0.35 if tool.startswith("partdesign.") else 0.08)

        return run_open(
            request,
            modules,
            on_step=show,
            calculix_path=resolve_calculix(),
            automatic_redesign=automatic_redesign,
        )


def show_dock() -> None:
    _prepare_path()
    window = Gui.getMainWindow()
    if window is None:
        return
    existing = window.findChild(QtWidgets.QDockWidget, "KalaDesign")
    if existing is None:
        existing = KalaDock(window)
        window.addDockWidget(_dock_area(), existing)
    existing.show()
    existing.raise_()


class _ShowCommand:
    def GetResources(self) -> dict[str, str]:
        return {"MenuText": "Design panel", "ToolTip": "Show the Kala design box"}

    def Activated(self) -> None:
        show_dock()

    def IsActive(self) -> bool:
        return True


class KalaWorkbench(Gui.Workbench):
    MenuText = "Kala"
    ToolTip = "Design a part from a request, in this window"

    def Initialize(self) -> None:
        Gui.addCommand("Kala_Design", _ShowCommand())
        self.appendToolbar("Kala", ["Kala_Design"])
        self.appendMenu("Kala", ["Kala_Design"])

    def Activated(self) -> None:
        show_dock()

    def GetClassName(self) -> str:
        return "Gui::PythonWorkbench"


def register() -> None:
    """Add the workbench and open the design box once the window exists."""
    try:
        Gui.getWorkbench("KalaWorkbench")
    except Exception:
        Gui.addWorkbench(KalaWorkbench())
    QtCore.QTimer.singleShot(800, _show_when_ready)


def _show_when_ready(attempt: int = 0) -> None:
    if Gui.getMainWindow() is None:
        if attempt < 20:
            QtCore.QTimer.singleShot(500, lambda: _show_when_ready(attempt + 1))
        return
    show_dock()
