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

    def __init__(
        self,
        prompt: str,
        analyze: bool,
        script: dict[str, object] | None,
        add_part: bool,
    ) -> None:
        super().__init__()
        self._prompt = prompt
        self._analyze = analyze
        self._script = script
        self._add_part = add_part

    def run(self) -> None:
        try:
            from kala_agent_freecad.cad.compose import continue_design, design_request

            if self._script is None:
                plan = design_request(
                    self._prompt, on_log=self.log_line.emit, analyze=self._analyze
                )
            else:
                plan = continue_design(
                    self._prompt,
                    self._script,
                    on_log=self.log_line.emit,
                    analyze=self._analyze,
                    add_part=self._add_part,
                )
            script = getattr(plan, "script", None)
            explanation = plan.summary
            try:
                from kala_agent_freecad.cad.models import explain_design

                source = json.dumps(script) if isinstance(script, dict) else plan.summary
                asked = self._prompt
                if self._script is not None:
                    asked = f"Change the current design: {self._prompt}"
                explanation = explain_design(asked, source)
            except Exception as error:
                explanation = f"{plan.summary}\n\n{error}"
            self.log_line.emit("Explanation:")
            self.log_line.emit(explanation)
            payload: dict[str, object] = {
                "analyze": self._analyze,
                "calls": plan.as_json(),
                "continued": self._script is not None,
                "explanation": explanation,
                "prompt": self._prompt,
                "summary": plan.summary,
            }
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
        self.setMinimumWidth(360)
        body = QtWidgets.QWidget(self)
        body.setObjectName("KalaBody")
        layout = QtWidgets.QVBoxLayout(body)
        layout.setContentsMargins(16, 14, 16, 14)
        layout.setSpacing(10)
        self._session = QtWidgets.QLabel("No design yet", body)
        self._session.setObjectName("KalaSession")
        self._prompt = QtWidgets.QPlainTextEdit(body)
        self._prompt.setObjectName("KalaPrompt")
        self._prompt.setPlaceholderText("a screw jack, or an 80 x 40 x 8 mm plate")
        self._prompt.setFixedHeight(116)
        _editor_margin(self._prompt)
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
        self._build.setObjectName("KalaBuild")
        self._build.setMinimumHeight(40)
        actions = QtWidgets.QGridLayout()
        actions.setHorizontalSpacing(8)
        actions.setVerticalSpacing(8)
        self._fresh = QtWidgets.QPushButton("New design", body)
        self._part = QtWidgets.QPushButton("New part", body)
        self._save = QtWidgets.QPushButton("Save", body)
        self._export = QtWidgets.QPushButton("Export STEP", body)
        for button in (self._fresh, self._part, self._save, self._export):
            button.setObjectName("KalaAction")
            button.setMinimumHeight(34)
        actions.addWidget(self._fresh, 0, 0)
        actions.addWidget(self._part, 0, 1)
        actions.addWidget(self._save, 1, 0)
        actions.addWidget(self._export, 1, 1)
        actions.setColumnStretch(0, 1)
        actions.setColumnStretch(1, 1)
        self._part.setEnabled(False)
        self._status = QtWidgets.QLabel("Write a request. The part is built in this window.", body)
        self._status.setObjectName("KalaStatus")
        self._status.setWordWrap(True)
        explanation_label = QtWidgets.QLabel("What was built", body)
        explanation_label.setObjectName("KalaSection")
        self._explanation = QtWidgets.QPlainTextEdit(body)
        self._explanation.setObjectName("KalaExplanation")
        self._explanation.setReadOnly(True)
        self._explanation.setMinimumHeight(96)
        self._explanation.setPlaceholderText(
            "After a build, the model explains the parts, sizes, and what it left out."
        )
        _editor_margin(self._explanation)
        _fill(self._explanation)
        log_label = QtWidgets.QLabel("Build log", body)
        log_label.setObjectName("KalaSection")
        self._log = QtWidgets.QPlainTextEdit(body)
        self._log.setObjectName("KalaLog")
        self._log.setReadOnly(True)
        self._log.setMinimumHeight(120)
        self._log.setPlaceholderText("The script, each decision, and each FreeCAD tool land here.")
        _editor_margin(self._log)
        _fill(self._log)
        layout.addWidget(self._session)
        layout.addWidget(self._prompt)
        if self._key is not None:
            layout.addWidget(self._key)
        if self._decide is not None:
            layout.addWidget(self._decide)
        layout.addWidget(self._analyze)
        layout.addWidget(self._build)
        layout.addLayout(actions)
        layout.addWidget(self._status)
        layout.addWidget(explanation_label)
        layout.addWidget(self._explanation, 2)
        layout.addWidget(log_label)
        layout.addWidget(self._log, 3)
        body.setStyleSheet(_STYLE)
        self.setWidget(body)
        self._worker: _Planner | None = None
        self._journal: object | None = None
        self._script: dict[str, object] | None = None
        self._build.clicked.connect(lambda: self._start(add_part=False))
        self._part.clicked.connect(lambda: self._start(add_part=True))
        self._fresh.clicked.connect(self._new_design)
        self._save.clicked.connect(self._save_document)
        self._export.clicked.connect(self._export_step)

    def _start(self, *, add_part: bool = False) -> None:
        prompt = self._prompt.toPlainText().strip()
        if not prompt:
            self._status.setText(
                "Describe the part to add." if add_part else "The request is empty."
            )
            return
        if add_part and self._script is None:
            self._status.setText("Build a design first, then add a part.")
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
        self._explanation.clear()
        self._open_journal(prompt)
        waiting, blocked = self._waiting()
        if blocked:
            self._status.setText(waiting)
            return
        self._set_busy(True)
        if self._script is None:
            self._status.setText("Writing a new design…")
        elif add_part:
            self._status.setText("Adding a part to this design…")
        else:
            self._status.setText("Changing this design…")
        worker = _Planner(prompt, self._analyze.isChecked(), self._script, add_part)
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
        self._set_busy(False)

    def _new_design(self) -> None:
        self._script = None
        self._prompt.clear()
        self._explanation.clear()
        self._refresh_session()
        self._append("Started a new design. The open model stays until the next build.")
        self._status.setText("Next request starts a new design. Save the open model if you still need it.")

    def _refresh_session(self) -> None:
        editing = self._script is not None
        self._session.setText("Editing this design" if editing else "No design yet")
        self._part.setEnabled(editing and self._build.isEnabled())
        self._build.setText("Apply change" if editing else "Build in this window")
        self._prompt.setPlaceholderText(
            "a 10 mm hole in the center, or make the plate 100 mm wide"
            if editing
            else "a screw jack, or an 80 x 40 x 8 mm plate"
        )

    def _set_busy(self, busy: bool) -> None:
        self._build.setEnabled(not busy)
        self._fresh.setEnabled(not busy)
        self._save.setEnabled(not busy)
        self._export.setEnabled(not busy)
        self._part.setEnabled(not busy and self._script is not None)

    def _save_document(self) -> None:
        import FreeCAD as App

        document = App.ActiveDocument
        if document is None:
            self._status.setText("There is no model to save.")
            return
        try:
            filename = str(getattr(document, "FileName", "") or "")
            if filename:
                document.save()
                self._status.setText(f"Saved {filename}")
                return
            path, _selected = QtWidgets.QFileDialog.getSaveFileName(
                self,
                "Save design",
                str(Path.home() / "kala.FCStd"),
                "FreeCAD (*.FCStd)",
            )
            if not path:
                return
            if not path.lower().endswith(".fcstd"):
                path = f"{path}.FCStd"
            document.saveAs(path)
            self._status.setText(f"Saved {path}")
        except Exception as error:
            self._status.setText(str(error))

    def _export_step(self) -> None:
        import FreeCAD as App

        document = App.ActiveDocument
        if document is None:
            self._status.setText("There is no model to export.")
            return
        path, _selected = QtWidgets.QFileDialog.getSaveFileName(
            self,
            "Export STEP",
            str(Path.home() / "kala.step"),
            "STEP (*.step *.stp)",
        )
        if not path:
            return
        if not path.lower().endswith((".step", ".stp")):
            path = f"{path}.step"
        bodies = [
            obj
            for obj in list(getattr(document, "Objects", ()) or ())
            if str(getattr(obj, "TypeId", "")) == "PartDesign::Body"
        ]
        if not bodies:
            self._status.setText("There is no solid to export.")
            return
        try:
            import Import

            Import.export(bodies, path)
        except Exception as error:
            self._status.setText(str(error))
            return
        self._status.setText(f"Exported {path}")

    def _build_calls(self, payload: str) -> None:
        try:
            parsed = json.loads(payload)
            summary = str(parsed.get("summary", ""))
            calls = json.dumps(parsed["calls"])
            explanation = parsed.get("explanation")
            if isinstance(explanation, str) and explanation.strip():
                self._explanation.setPlainText(explanation.strip())
            self._append("FreeCAD")
            self._status.setText(summary or "Building in this window.")
            measurements = self._run(calls, automatic_redesign=False)
            script = parsed.get("script")
            if isinstance(script, dict):
                self._script = script
            self._refresh_session()
            if parsed.get("analyze") and isinstance(parsed.get("script"), dict):
                self._revise(
                    parsed,
                    {
                        "analysis_detail": measurements.get("analysis_detail", ""),
                        "analysis_status": measurements.get("analysis_status", ""),
                        "peak_stress_mpa": measurements.get("peak_stress_mpa"),
                    },
                )
            if parsed.get("continued"):
                self._status.setText(summary or "Updated this design.")
            else:
                self._status.setText(summary or "The model is open in this window.")
        except Exception as error:
            self._append(str(error))
            self._status.setText(str(error))
        finally:
            self._set_busy(False)

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
        gui = Gui.ActiveDocument
        if gui is not None:
            try:
                gui.resetEdit()
            except Exception:
                pass
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
            reuse_document=True,
        )


def _editor_margin(editor: QtWidgets.QPlainTextEdit) -> None:
    editor.document().setDocumentMargin(8)
    hidden = QtCore.Qt.ScrollBarPolicy.ScrollBarAlwaysOff
    editor.setHorizontalScrollBarPolicy(hidden)
    editor.setLineWrapMode(QtWidgets.QPlainTextEdit.LineWrapMode.WidgetWidth)


def _fill(editor: QtWidgets.QPlainTextEdit) -> None:
    policy = QtWidgets.QSizePolicy.Policy
    editor.setSizePolicy(policy.Expanding, policy.Expanding)


_STYLE = """
QWidget#KalaBody {
  background: #f6f5f2;
  color: #1c1b19;
  font-size: 14px;
}
QLabel#KalaSession {
  color: #1f6b4a;
  font-size: 13px;
  font-weight: 600;
}
QLabel#KalaStatus {
  color: #5e584f;
  font-size: 13px;
}
QLabel#KalaSection {
  color: #5e584f;
  font-size: 12px;
  font-weight: 600;
}
QPlainTextEdit {
  background: #ffffff;
  color: #1c1b19;
  border: 1px solid #e4e0d8;
  border-radius: 8px;
  font-size: 14px;
  selection-background-color: #d7efe3;
  selection-color: #1c1b19;
}
QPlainTextEdit#KalaPrompt {
  font-size: 15px;
}
QPlainTextEdit#KalaLog {
  background: #fbfaf8;
  color: #3f3c38;
  font-family: "Source Code Pro", monospace;
  font-size: 12px;
}
QPlainTextEdit QScrollBar:vertical {
  background: transparent;
  width: 8px;
  margin: 4px 2px;
}
QPlainTextEdit QScrollBar::handle:vertical {
  background: #d9d3c9;
  border-radius: 4px;
  min-height: 24px;
}
QPlainTextEdit QScrollBar::add-line:vertical,
QPlainTextEdit QScrollBar::sub-line:vertical,
QPlainTextEdit QScrollBar::add-page:vertical,
QPlainTextEdit QScrollBar::sub-page:vertical {
  background: transparent;
  height: 0;
}
QLineEdit {
  background: #ffffff;
  color: #1c1b19;
  border: 1px solid #e4e0d8;
  border-radius: 8px;
  padding: 8px 10px;
  min-height: 22px;
}
QCheckBox {
  color: #1c1b19;
  spacing: 8px;
}
QPushButton#KalaBuild {
  background: #1f6b4a;
  color: #ffffff;
  border: none;
  border-radius: 8px;
  padding: 8px 14px;
  font-size: 14px;
  font-weight: 600;
}
QPushButton#KalaBuild:disabled {
  background: #9aab9f;
  color: #ffffff;
}
QPushButton#KalaAction {
  background: #ffffff;
  color: #1c1b19;
  border: 1px solid #e0dbd3;
  border-radius: 8px;
  padding: 6px 10px;
  font-size: 13px;
}
QPushButton#KalaAction:disabled {
  color: #a39e96;
  background: #f3f1ec;
}
"""


def show_dock() -> None:
    _prepare_path()
    window = Gui.getMainWindow()
    if window is None:
        return
    existing = window.findChild(QtWidgets.QDockWidget, "KalaDesign")
    created = existing is None
    if created:
        existing = KalaDock(window)
        window.addDockWidget(_dock_area(), existing)
        resize = getattr(window, "resizeDocks", None)
        orientation = getattr(QtCore.Qt, "Orientation", QtCore.Qt)
        if callable(resize):
            resize([existing], [400], orientation.Horizontal)
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
