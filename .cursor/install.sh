#!/usr/bin/env bash
# Idempotent repository bootstrap for the Kala-Agent Cloud Agent environment.
# Safe to run repeatedly and against a partially prepared or cached workspace.
set -euo pipefail

cd "$(dirname "$0")/.."

# The default base image ships CPython but not the stdlib venv/ensurepip
# support, so provide it before creating the project virtual environment.
if ! python3 -c "import ensurepip" >/dev/null 2>&1; then
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3-venv
fi

# Create the project virtual environment (matches .gitignore's .venv/).
if [ ! -x .venv/bin/python ]; then
  python3 -m venv .venv
fi

# Install Kala-Agent in editable mode with the dev toolchain (pytest, ruff, mypy).
.venv/bin/python -m pip install --upgrade pip
.venv/bin/python -m pip install -e ".[dev]"

echo "Kala-Agent environment ready. Activate with: source .venv/bin/activate"
