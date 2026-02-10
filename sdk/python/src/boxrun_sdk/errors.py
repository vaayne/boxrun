"""BoxRun error types."""

from __future__ import annotations


class BoxRunError(Exception):
    """Error returned by the BoxRun server.

    Attributes:
        code: Error code string (e.g. ``"BOX_NOT_FOUND"``).
        message: Human-readable error description.
    """

    def __init__(self, code: str, message: str) -> None:
        self.code = code
        self.message = message
        super().__init__(f"[{code}] {message}")
