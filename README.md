# Kala-Agent

An AI-powered CAD agent with a modular architecture — shared core intelligence with swappable CAD kernel backends.

## Architecture

```text
+--------------------+
|    Kala-Agent      |
|  (Core AI Brain)   |
+---------+----------+
          |
    +-----+-----+
    |           |
+---v---+   +---v---+
| OCP   |   | Other |
| (1st) |   | ...   |
+-------+   +-------+
```

## Features
- Modular architecture with shared AI core
- Swappable CAD backends
- Common geometry interfaces

## Project Structure
- `kala/core`: Shared core intelligence and geometry definitions
- `kala/backends`: CAD backend implementations
- `kala/utils`: Shared utilities

## Getting Started
To get started, clone the repository and install the dependencies.

## Development
Run tests using pytest.

## License
MIT License
