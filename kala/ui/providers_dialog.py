"""OpenRouter API provider dialog: fetch and list available models."""

from __future__ import annotations

from PySide6.QtCore import Qt, QThread, Signal
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from kala.llm.model_catalog import ModelFetchError, fetch_openrouter_models
from kala.llm.providers import (
    ProviderConfig,
    ProviderStore,
    config_path,
    load_store,
    new_from_preset,
    save_store,
)

OPENROUTER_BASE = "https://openrouter.ai/api/v1"


class OpenRouterFetchWorker(QThread):
    finished_ok = Signal(object)  # list[dict]
    failed = Signal(str)

    def __init__(self, api_key: str) -> None:
        super().__init__()
        self.api_key = api_key

    def run(self) -> None:
        try:
            models = fetch_openrouter_models(
                api_key=self.api_key, base_url=OPENROUTER_BASE
            )
            payload = [{"id": m.id, "name": m.name, "label": m.label} for m in models]
            self.finished_ok.emit(payload)
        except ModelFetchError as exc:
            self.failed.emit(str(exc))
        except Exception as exc:  # noqa: BLE001
            self.failed.emit(str(exc))


class ProvidersDialog(QDialog):
    """OpenRouter-only provider setup with live model catalog."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("OpenRouter")
        self.resize(640, 560)
        self.store: ProviderStore = load_store()
        self._models: list[dict] = []
        self._worker: OpenRouterFetchWorker | None = None

        # Ensure one OpenRouter entry exists
        self._cfg = self._ensure_openrouter()

        self.setStyleSheet(
            """
            QDialog { background: #111314; color: #c9ced4; }
            QLabel { color: #c9ced4; }
            QLabel#muted { color: #6b7280; font-size: 12px; }
            QLineEdit {
              background: #16191c; color: #e2e5e9;
              border: 1px solid #2a2f35; border-radius: 2px; padding: 7px 10px;
            }
            QLineEdit:focus { border-color: #4d6b58; }
            QListWidget {
              background: #16191c; color: #c9ced4;
              border: 1px solid #2a2f35; border-radius: 2px;
              outline: none;
            }
            QListWidget::item { padding: 6px 8px; }
            QListWidget::item:selected { background: #2f3f36; color: #e8ece9; }
            QPushButton {
              background: #16191c; color: #a8b0b8;
              border: 1px solid #2a2f35; border-radius: 2px; padding: 6px 12px;
            }
            QPushButton:hover { color: #e2e5e9; border-color: #3a4048; }
            """
        )

        layout = QVBoxLayout(self)
        layout.setContentsMargins(20, 18, 20, 18)
        layout.setSpacing(12)

        hint = QLabel(
            "OpenRouter API key. Models load from the catalog.\n"
            f"{config_path()}"
        )
        hint.setObjectName("muted")
        hint.setWordWrap(True)
        layout.addWidget(hint)

        form = QFormLayout()
        self.name_edit = QLineEdit(self._cfg.name or "OpenRouter")
        self.api_key_edit = QLineEdit(self._cfg.api_key)
        self.api_key_edit.setEchoMode(QLineEdit.EchoMode.Password)
        self.api_key_edit.setPlaceholderText("sk-or-v1-…")
        self.api_key_edit.editingFinished.connect(self._fetch_models)
        form.addRow("Name", self.name_edit)
        form.addRow("API key", self.api_key_edit)
        layout.addLayout(form)

        filter_row = QHBoxLayout()
        self.model_filter = QLineEdit()
        self.model_filter.setPlaceholderText("Filter models (e.g. claude, gpt, llama)…")
        self.model_filter.textChanged.connect(self._repopulate_list)
        self.refresh_btn = QPushButton("Refresh models")
        self.refresh_btn.setObjectName("secondary")
        self.refresh_btn.clicked.connect(lambda: self._fetch_models())
        filter_row.addWidget(self.model_filter, 1)
        filter_row.addWidget(self.refresh_btn)
        layout.addLayout(filter_row)

        self.models_status = QLabel("Loading OpenRouter models…")
        self.models_status.setObjectName("muted")
        layout.addWidget(self.models_status)

        self.model_list = QListWidget()
        self.model_list.setAlternatingRowColors(True)
        layout.addWidget(self.model_list, 1)

        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Save | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(self._on_save)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

        # Load catalog immediately (OpenRouter allows unauthenticated list)
        self._fetch_models()

    def _ensure_openrouter(self) -> ProviderConfig:
        for cfg in self.store.providers:
            if cfg.provider == "openrouter":
                self.store.active_id = cfg.id
                return cfg
        cfg = new_from_preset("openrouter", "OpenRouter")
        self.store.upsert(cfg)
        self.store.active_id = cfg.id
        return cfg

    def _fetch_models(self) -> None:
        if self._worker and self._worker.isRunning():
            return
        self.refresh_btn.setEnabled(False)
        self.models_status.setText("Fetching models from OpenRouter…")
        self._worker = OpenRouterFetchWorker(self.api_key_edit.text().strip())
        self._worker.finished_ok.connect(self._on_loaded)
        self._worker.failed.connect(self._on_failed)
        self._worker.start()

    def _on_loaded(self, payload: object) -> None:
        self.refresh_btn.setEnabled(True)
        models = list(payload) if isinstance(payload, list) else []
        self._models = [m for m in models if isinstance(m, dict) and m.get("id")]
        self._repopulate_list()
        self.models_status.setText(f"{len(self._models)} models from OpenRouter")

    def _on_failed(self, err: str) -> None:
        from kala.ui.error_copy import provider_fail_copy

        self.refresh_btn.setEnabled(True)
        friendly = provider_fail_copy(err)
        self.models_status.setText(friendly)
        QMessageBox.warning(self, "OpenRouter", friendly)

    def _repopulate_list(self, _text: str = "") -> None:
        needle = self.model_filter.text().strip().lower()
        selected = self._cfg.model
        current = self.model_list.currentItem()
        if current is not None:
            selected = str(current.data(Qt.ItemDataRole.UserRole) or selected)

        self.model_list.clear()
        shown = 0
        select_row = 0
        for model in self._models:
            mid = str(model.get("id") or "")
            label = str(model.get("label") or mid)
            hay = f"{mid} {model.get('name') or ''}".lower()
            if needle and needle not in hay:
                continue
            item = QListWidgetItem(label)
            item.setData(Qt.ItemDataRole.UserRole, mid)
            self.model_list.addItem(item)
            if mid == selected:
                select_row = shown
            shown += 1

        if self.model_list.count():
            self.model_list.setCurrentRow(select_row)
        if self._models:
            self.models_status.setText(
                f"Showing {shown} of {len(self._models)} OpenRouter models"
                + (f" · “{self.model_filter.text().strip()}”" if needle else "")
            )

    def _selected_model_id(self) -> str:
        item = self.model_list.currentItem()
        if item is not None:
            return str(item.data(Qt.ItemDataRole.UserRole) or "")
        return self._cfg.model

    def _on_save(self) -> None:
        from kala.llm.providers import looks_like_api_key

        model_id = self._selected_model_id()
        if not model_id and self._models:
            QMessageBox.warning(self, "OpenRouter", "Select a model from the list.")
            return
        key = self.api_key_edit.text().strip()
        if key and not looks_like_api_key(key, "openrouter"):
            QMessageBox.warning(
                self,
                "API key",
                "That looks like a website URL, not an API key.\n\n"
                "Paste your key from https://openrouter.ai/keys "
                "(it usually starts with sk-or-…).\n"
                "Base URL stays https://openrouter.ai/api/v1; that is not the key.",
            )
            return
        if not key:
            QMessageBox.warning(
                self,
                "API key",
                "Paste your OpenRouter API key (sk-or-…) to use the LLM planner.\n"
                "Without a key, Kala falls back to stub heuristics.",
            )
        updated = ProviderConfig(
            id=self._cfg.id,
            name=self.name_edit.text().strip() or "OpenRouter",
            provider="openrouter",
            api_key=key,
            base_url=OPENROUTER_BASE,
            model=model_id or self._cfg.model,
            enabled=True,
        )
        # Keep only OpenRouter for now
        self.store.providers = [updated]
        self.store.active_id = updated.id
        path = save_store(self.store)
        self._cfg = updated
        QMessageBox.information(
            self,
            "Saved",
            f"OpenRouter saved.\nModel: {updated.model}\n{path}",
        )
        self.accept()
