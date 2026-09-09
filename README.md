# Kala-Agent

CAD design agent with context-aware planning.

## Features

- **Design Context Models**: Pluggable context enrichment for agent planning
- **Stub Mode** (default): No-op fallback for baseline behavior
- **Learned Mode**: Rule-based context enrichment from hand-authored artifacts

## Installation

```bash
pip install -e .
```

For development with tests:

```bash
pip install -e ".[test]"
```

## Usage

### CLI

Run the agent with default stub context model:

```bash
kala --backend mock --json
```

Run with learned context model:

```bash
KALA_CONTEXT_MODEL=learned kala --backend mock --json
```

### Environment Variables

- `KALA_CONTEXT_MODEL`: Select context model implementation
  - `stub` (default): No-op fallback with default DynamicContext values
  - `learned`: Rule-based enrichment from `kala/ml/artifacts/context_v0.json`

### Python API

```python
from kala.agent.loop import Agent, get_context_model
from kala.ml.stub import StubDesignContextModel
from kala.ml.learned import LearnedDesignContextModel

# Use environment-based factory
agent = Agent()
agent.plan_with_context()

# Or inject specific model
agent = Agent(context_model=LearnedDesignContextModel())
agent.plan_with_context()
```

## Architecture

### Context Enrichment Flow

1. **Agent State** → `extract_features()` → Feature Dictionary
2. Feature Dictionary → `LearnedDesignContextModel.enrich()` → Rule Evaluation
3. Rule Evaluation → **DynamicContext** (focus, constraints_active, export_ready, search_density)
4. DynamicContext consumed by planner at `kala/agent/loop.py` ~L158

### DynamicContext Fields

- `focus`: Planning mode (`"exploration"`, `"refinement"`, `"validation"`)
- `constraints_active`: Boolean flag for constraint handling
- `export_ready`: Boolean flag for export workflow readiness
- `search_density`: Float [0.0, 1.0] indicating exploration vs exploitation balance

### Fallback Behavior

`LearnedDesignContextModel` automatically falls back to `StubDesignContextModel` when:

- Artifact file `context_v0.json` is missing
- Artifact JSON is malformed
- Artifact contains no rules

## Development

### Run Tests

```bash
pytest tests/test_context_models.py -v
```

Tests cover:

- Protocol compliance (both models implement `DesignContextModel`)
- Empty history handling (no crash, valid output)
- Fallback behavior (missing/invalid artifacts)
- Feature extraction correctness
- Rule evaluation logic

### Project Structure

```
kala/
├── agent/
│   ├── __init__.py
│   └── loop.py              # Agent orchestration, enrich call site ~L158
├── ml/
│   ├── __init__.py
│   ├── base.py              # DesignContextModel protocol, DynamicContext
│   ├── stub.py              # StubDesignContextModel (default fallback)
│   ├── learned.py           # LearnedDesignContextModel (rule-based)
│   ├── features.py          # extract_features(state) -> dict
│   └── artifacts/
│       └── context_v0.json  # Hand-authored rules
├── cli.py                   # CLI entry point
└── __init__.py

tests/
└── test_context_models.py   # Unit tests
```

## CTX-0 Implementation

This implementation follows the CTX-0 specification:

- ✅ **Protocol unchanged**: `DesignContextModel.enrich(state) -> DynamicContext`
- ✅ **Stub fallback preserved**: `StubDesignContextModel` remains default
- ✅ **Feature extraction**: `extract_features()` with body/tool counts, search hits, etc.
- ✅ **Learned model**: Loads `context_v0.json`, falls back on missing/invalid artifact
- ✅ **Environment control**: `KALA_CONTEXT_MODEL=stub|learned` (default `stub`)
- ✅ **Unit tests**: Protocol, empty history, fallback coverage
- ✅ **No breaking changes**: Loop signature at ~L158 unchanged, no planner rewrites

## License

[Specify license]
