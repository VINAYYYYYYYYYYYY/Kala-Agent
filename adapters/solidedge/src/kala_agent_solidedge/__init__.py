"""Windows-local Solid Edge client for Kala Agent.

The package compiles a shared :class:`kala_agent.contracts.ConstructionScript`
into bridge protocol v1 operations, predicts the measured parity corpus, and
speaks authenticated loopback HTTP to the local ``kala-se-bridge`` service.

Nothing here imports ``FreeCAD``, the Kala studio application, the kernel, the
tool plane, or any COM module. The target runtime is Windows; importing this
package on a development host is supported for pure compiler and client tests.
"""

from __future__ import annotations

from .compile import build_request, compile_operations, new_request_id
from .errors import (
    ArtifactError,
    AuthenticationError,
    ClientError,
    CompileError,
    NetworkError,
    ProtocolError,
    SolidEdgeClientError,
)

__all__ = (
    "ArtifactError",
    "AuthenticationError",
    "ClientError",
    "CompileError",
    "NetworkError",
    "ProtocolError",
    "SolidEdgeClientError",
    "build_request",
    "compile_operations",
    "new_request_id",
)
