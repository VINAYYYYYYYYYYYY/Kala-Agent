"""Typed bridge failures that map directly onto :class:`ErrorResponse`."""

from __future__ import annotations

from .protocol import ErrorCode, ErrorResponse

_MAX_MESSAGE_CHARS = 500


class BridgeError(Exception):
    """A failure with a stable wire code. Importing this never touches Windows."""

    def __init__(self, code: ErrorCode, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message

    def to_response(self) -> ErrorResponse:
        """Return the bounded wire representation of this failure."""
        message = self.message.strip()[:_MAX_MESSAGE_CHARS] or "bridge failure"
        return ErrorResponse(code=self.code, message=message)


class InputError(BridgeError):
    """The request was refused before any Solid Edge automation was attempted."""

    def __init__(self, message: str, code: ErrorCode = ErrorCode.INVALID_REQUEST) -> None:
        super().__init__(code, message)


class UnavailableError(BridgeError):
    """Solid Edge, COM, or the calibrated worker environment is not available."""

    def __init__(self, message: str) -> None:
        super().__init__(ErrorCode.UNAVAILABLE, message)


class CalibrationRequiredError(BridgeError):
    """The host has not been calibrated for the requested native operation."""

    def __init__(self, message: str) -> None:
        super().__init__(ErrorCode.CALIBRATION_REQUIRED, message)


class BusyError(BridgeError):
    """Another build owns the single COM worker."""

    def __init__(self, message: str) -> None:
        super().__init__(ErrorCode.BUSY, message)


class ConflictError(BridgeError):
    """The build identity or target conflicts with existing bridge state."""

    def __init__(self, message: str) -> None:
        super().__init__(ErrorCode.CONFLICT, message)


__all__ = (
    "BridgeError",
    "BusyError",
    "CalibrationRequiredError",
    "ConflictError",
    "InputError",
    "UnavailableError",
)
