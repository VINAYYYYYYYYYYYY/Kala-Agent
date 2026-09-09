"""Kala Desktop UI - Main window."""
import sys
import json
import subprocess
from pathlib import Path
from typing import Optional

from PyQt6.QtWidgets import (
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QTextEdit,
    QMessageBox,
)
from PyQt6.QtCore import Qt, QThread, pyqtSignal
from PyQt6.QtGui import QFont

from kala.llm.providers import load_store, looks_like_api_key, get_planner_key
from kala.ui.providers_dialog import ProvidersDialog


# Starter prompts - L-bracket MUST be first with exact golden brief
STARTERS = [
    {
        "label": "L-bracket",
        "prompt": "L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step"
    },
    {
        "label": "Simple Box",
        "prompt": "Create a simple box 100x100x50mm at origin, export box.step"
    },
    {
        "label": "Cylinder",
        "prompt": "Create a cylinder radius 25mm height 80mm at origin, export cylinder.step"
    },
]


class RunWorker(QThread):
    """Worker thread for running kala agent."""
    
    finished = pyqtSignal(dict)
    failed = pyqtSignal(str, str)
    
    def __init__(self, prompt: str, procedure: str = "simple_bracket", parts: bool = False):
        super().__init__()
        self.prompt = prompt
        self.procedure = procedure
        self.parts = parts
    
    def run(self):
        """Execute kala run command."""
        try:
            cmd = [
                "kala",
                "run",
                "--json",
                "--prompt", self.prompt,
                "--procedure", self.procedure,
            ]
            if self.parts:
                cmd.append("--parts")
            
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=300
            )
            
            if result.returncode != 0:
                self.failed.emit("nonzero", result.stderr)
                return
            
            try:
                output = json.loads(result.stdout)
                self.finished.emit(output)
            except json.JSONDecodeError as e:
                self.failed.emit("json_parse", str(e))
        
        except subprocess.TimeoutExpired:
            self.failed.emit("timeout", "Operation timed out after 5 minutes")
        except FileNotFoundError:
            self.failed.emit("freecad_missing", "FreeCAD not found")
        except Exception as e:
            self.failed.emit("unknown", str(e))


class KalaDesktopUI(QMainWindow):
    """Main Kala desktop application window."""
    
    def __init__(self):
        super().__init__()
        self.worker: Optional[RunWorker] = None
        self.setWindowTitle("Kala CAD Agent")
        self.setMinimumSize(800, 600)
        self._setup_ui()
        self._refresh_planner()
    
    def _setup_ui(self):
        """Setup the main UI."""
        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)
        
        # Header
        header_layout = QHBoxLayout()
        
        title = QLabel("Kala")
        title_font = QFont()
        title_font.setPointSize(18)
        title_font.setBold(True)
        title.setFont(title_font)
        header_layout.addWidget(title)
        
        header_layout.addStretch()
        
        # Planner status label
        self.planner_lbl = QLabel()
        planner_font = QFont()
        planner_font.setPointSize(10)
        self.planner_lbl.setFont(planner_font)
        header_layout.addWidget(self.planner_lbl)
        
        # Providers button
        providers_btn = QPushButton("⚙️ Providers")
        providers_btn.clicked.connect(self._open_providers)
        header_layout.addWidget(providers_btn)
        
        layout.addLayout(header_layout)
        
        # Starters section
        starters_label = QLabel("Quick Starts:")
        starters_font = QFont()
        starters_font.setPointSize(12)
        starters_font.setBold(True)
        starters_label.setFont(starters_font)
        layout.addWidget(starters_label)
        
        # Starter chips
        chips_scroll = QScrollArea()
        chips_scroll.setWidgetResizable(True)
        chips_scroll.setMaximumHeight(80)
        
        chips_widget = QWidget()
        chips_layout = QHBoxLayout(chips_widget)
        chips_layout.setContentsMargins(0, 0, 0, 0)
        
        for starter in STARTERS:
            chip_btn = QPushButton(starter["label"])
            chip_btn.setStyleSheet("""
                QPushButton {
                    background-color: #e0e0e0;
                    border: none;
                    border-radius: 12px;
                    padding: 8px 16px;
                    font-size: 11px;
                }
                QPushButton:hover {
                    background-color: #d0d0d0;
                }
            """)
            chip_btn.clicked.connect(lambda checked, p=starter["prompt"]: self._fill(p))
            chips_layout.addWidget(chip_btn)
        
        chips_layout.addStretch()
        chips_scroll.setWidget(chips_widget)
        layout.addWidget(chips_scroll)
        
        # Prompt input
        prompt_label = QLabel("Prompt:")
        layout.addWidget(prompt_label)
        
        self.prompt_input = QTextEdit()
        self.prompt_input.setPlaceholderText("Describe your CAD model...")
        self.prompt_input.setMaximumHeight(120)
        layout.addWidget(self.prompt_input)
        
        # Run button
        self.run_btn = QPushButton("▶️ Run")
        self.run_btn.setStyleSheet("""
            QPushButton {
                background-color: #4CAF50;
                color: white;
                border: none;
                border-radius: 4px;
                padding: 10px;
                font-size: 14px;
                font-weight: bold;
            }
            QPushButton:hover {
                background-color: #45a049;
            }
            QPushButton:disabled {
                background-color: #cccccc;
                color: #666666;
            }
        """)
        self.run_btn.clicked.connect(self._send)
        layout.addWidget(self.run_btn)
        
        # Output area
        output_label = QLabel("Output:")
        layout.addWidget(output_label)
        
        self.output_text = QTextEdit()
        self.output_text.setReadOnly(True)
        layout.addWidget(self.output_text)
    
    def _has_valid_planner_key(self) -> bool:
        """Check if a valid planner API key is configured.
        
        Returns:
            True if a valid key exists
        """
        key = get_planner_key()
        return looks_like_api_key(key)
    
    def _refresh_planner(self):
        """Refresh planner status and update UI."""
        if self._has_valid_planner_key():
            store = load_store()
            model = store.get("planner_model", "gpt-4")
            self.planner_lbl.setText(f"Planner: {model}")
            self.planner_lbl.setStyleSheet("color: green;")
            self.run_btn.setEnabled(True)
        else:
            self.planner_lbl.setText("Planner: no API key")
            self.planner_lbl.setStyleSheet("color: red;")
            self.run_btn.setEnabled(False)
    
    def _open_providers(self):
        """Open the providers configuration dialog."""
        dialog = ProvidersDialog(self)
        dialog.saved.connect(self._refresh_planner)
        dialog.exec()
    
    def _fill(self, prompt: str):
        """Fill prompt input with a starter prompt.
        
        Args:
            prompt: The prompt text to fill
        """
        self.prompt_input.setPlainText(prompt)
    
    def _send(self):
        """Send the prompt to the agent."""
        # Gate: check for valid API key before running
        if not self._has_valid_planner_key():
            QMessageBox.warning(
                self,
                "No API Key",
                "Planner needs an API key. Please configure it in Providers settings."
            )
            return
        
        prompt = self.prompt_input.toPlainText().strip()
        if not prompt:
            QMessageBox.warning(self, "Empty Prompt", "Please enter a prompt.")
            return
        
        if self.worker and self.worker.isRunning():
            QMessageBox.warning(self, "Already Running", "A job is already running.")
            return
        
        self.run_btn.setEnabled(False)
        self.output_text.setPlainText("Running...")
        
        # Keep defaults: procedure=simple_bracket, parts=False
        self.worker = RunWorker(prompt, procedure="simple_bracket", parts=False)
        self.worker.finished.connect(self._on_success)
        self.worker.failed.connect(self._fail)
        self.worker.start()
    
    def _on_success(self, result: dict):
        """Handle successful run.
        
        Args:
            result: JSON result from kala run
        """
        self.run_btn.setEnabled(True)
        output = json.dumps(result, indent=2)
        self.output_text.setPlainText(f"Success!\n\n{output}")
    
    def _fail(self, reason: str, detail: str):
        """Handle failed run with friendly error messages.
        
        Args:
            reason: Failure reason code
            detail: Technical detail (logged but not always shown)
        """
        self.run_btn.setEnabled(True)
        
        # Map failures to friendly UI messages
        error_map = {
            "missing_key": {
                "title": "No API Key",
                "body": "Please configure your API key in Providers settings."
            },
            "url_as_key": {
                "title": "Invalid API Key",
                "body": "The API key appears to be a URL. Please use your actual API key."
            },
            "freecad_missing": {
                "title": "FreeCAD Not Found",
                "body": "FreeCAD is required but not installed. Please install FreeCAD."
            },
            "sync_stale": {
                "title": "Synchronization Error",
                "body": "The agent state is out of sync. Please try again."
            },
            "timeout": {
                "title": "Timeout",
                "body": "The operation took too long and was cancelled."
            },
            "nonzero": {
                "title": "Execution Failed",
                "body": "The agent encountered an error during execution."
            },
            "json_parse": {
                "title": "Parse Error",
                "body": "Could not parse the agent output. Please try again."
            },
            "stub": {
                "title": "Stub Mode",
                "body": "Agent is in stub mode. Full implementation coming soon."
            }
        }
        
        # Get mapped error or create fallback
        if reason in error_map:
            error_info = error_map[reason]
            title = error_info["title"]
            body = error_info["body"]
        else:
            title = "Error"
            # Truncate detail to 140 chars for UI
            safe_detail = detail[:140] if detail and len(detail) > 140 else (detail or "Unknown error")
            # Filter out any API key patterns
            safe_detail = self._sanitize_error(safe_detail)
            body = safe_detail
        
        # Log full detail for debugging (without exposing in UI)
        print(f"Error [{reason}]: {detail}")
        
        # Show friendly message in UI
        QMessageBox.critical(self, title, body)
        self.output_text.setPlainText(f"Failed: {title}\n{body}")
    
    def _sanitize_error(self, text: str) -> str:
        """Remove API keys and sensitive data from error text.
        
        Args:
            text: Raw error text
            
        Returns:
            Sanitized text safe for UI display
        """
        # Remove common API key patterns
        import re
        patterns = [
            r'sk-[a-zA-Z0-9]{20,}',
            r'Authorization:\s*\S+',
            r'api[-_]?key[:=]\s*\S+',
        ]
        
        sanitized = text
        for pattern in patterns:
            sanitized = re.sub(pattern, '[REDACTED]', sanitized, flags=re.IGNORECASE)
        
        return sanitized


def main():
    """Launch the Kala desktop UI."""
    app = QApplication(sys.argv)
    window = KalaDesktopUI()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
