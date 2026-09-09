# Kala Agent

CAD design agent with desktop UI.

## Installation

```bash
pip install -e .
```

## Desktop UI

Launch the desktop interface:

```bash
kala-ui
```

### Features

1. **Provider Configuration**: Configure your LLM provider API key via the Providers dialog (⚙️ button)
2. **Quick Starts**: Pre-configured starter prompts including the L-bracket example
3. **API Key Gating**: Run button is disabled until a valid API key is configured
4. **Friendly Error Messages**: Clear, user-friendly error messages without technical details

### First Time Setup

1. Launch `kala-ui`
2. Click the **⚙️ Providers** button
3. Enter your OpenAI API key (or compatible provider)
4. Click **Save**
5. The Run button will become enabled

### Quick Start Examples

Click any starter chip to populate the prompt input:

- **L-bracket**: Creates an L-shaped bracket with mounting holes
- **Simple Box**: Creates a basic rectangular box
- **Cylinder**: Creates a cylindrical shape

## CLI

```bash
kala run --prompt "your CAD description" --json
```

## Development

The project structure:

- `kala/ui/` - Desktop UI components
- `kala/llm/` - LLM provider management
- `kala/cli.py` - Command-line interface
