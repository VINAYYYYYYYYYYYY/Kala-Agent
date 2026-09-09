"""Provider configuration dialog."""
from PyQt6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QMessageBox,
)
from PyQt6.QtCore import Qt, pyqtSignal

from kala.llm.providers import load_store, save_store, looks_like_api_key


class ProvidersDialog(QDialog):
    """Dialog for configuring LLM provider API keys."""
    
    saved = pyqtSignal()
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Provider Configuration")
        self.setMinimumWidth(500)
        self._setup_ui()
        self._load_current()
    
    def _setup_ui(self):
        """Setup the dialog UI."""
        layout = QVBoxLayout(self)
        
        # Planner API key
        planner_layout = QVBoxLayout()
        planner_label = QLabel("Planner API Key:")
        planner_layout.addWidget(planner_label)
        
        self.planner_input = QLineEdit()
        self.planner_input.setPlaceholderText("sk-... or provider API key")
        self.planner_input.setEchoMode(QLineEdit.EchoMode.Password)
        planner_layout.addWidget(self.planner_input)
        
        help_label = QLabel("Enter your OpenAI or compatible provider API key")
        help_label.setStyleSheet("color: gray; font-size: 11px;")
        planner_layout.addWidget(help_label)
        
        layout.addLayout(planner_layout)
        layout.addStretch()
        
        # Buttons
        button_layout = QHBoxLayout()
        button_layout.addStretch()
        
        cancel_btn = QPushButton("Cancel")
        cancel_btn.clicked.connect(self.reject)
        button_layout.addWidget(cancel_btn)
        
        save_btn = QPushButton("Save")
        save_btn.setDefault(True)
        save_btn.clicked.connect(self._save)
        button_layout.addWidget(save_btn)
        
        layout.addLayout(button_layout)
    
    def _load_current(self):
        """Load current configuration."""
        store = load_store()
        api_key = store.get("planner_api_key", "")
        if api_key:
            self.planner_input.setText(api_key)
    
    def _save(self):
        """Save configuration."""
        key = self.planner_input.text().strip()
        
        if key and not looks_like_api_key(key):
            QMessageBox.warning(
                self,
                "Invalid API Key",
                "The API key doesn't look valid. Please check the format."
            )
            return
        
        store = load_store()
        store["planner_api_key"] = key
        save_store(store)
        
        self.saved.emit()
        self.accept()
