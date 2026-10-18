"""Mocked-HTTP tests for the OpenAI/OpenRouter studio provider.

The tests exercise the real ``openai_compat`` code path with a fake
``urllib`` opener, so no socket is ever opened and no key reaches the
network. They pin the transport contract: fixed endpoint, bearer header,
consent-gated body, JSON parsing, bounded response, redirect refusal, and
typed error mapping.
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from collections.abc import Iterator, Mapping
from typing import Any

import pytest

from kala_agent.studio.providers.base import (
    ProviderConfigurationError,
    ProviderMissingCredentialError,
    ProviderNetworkError,
    ProviderProtocolError,
    ProviderTimeoutError,
    TooLargeResponseError,
)
from kala_agent.studio.providers.openai_compat import (
    MAX_HTTP_RESPONSE_BYTES,
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
    STUDIO_REQUEST_TIMEOUT_SECONDS,
    NoRedirectHandler,
    openai_compat_chat,
)

OPENAI_KEY = "sk-unit-test-openai-key"
OPENROUTER_KEY = "sk-unit-test-openrouter-key"


class _FakeResponse:
    def __init__(self, body: bytes, headers: Mapping[str, str] | None = None) -> None:
        self._body = body
        self.headers = dict(headers or {})
        self._pos = 0

    def read(self, size: int = -1) -> bytes:
        if size is None or size < 0:
            chunk = self._body[self._pos :]
            self._pos = len(self._body)
            return chunk
        chunk = self._body[self._pos : self._pos + size]
        self._pos += len(chunk)
        return chunk

    def __enter__(self) -> _FakeResponse:
        return self

    def __exit__(self, *_args: object) -> bool:
        return False


class _FakeOpener:
    def __init__(
        self,
        *,
        response: _FakeResponse | None = None,
        error: BaseException | None = None,
    ) -> None:
        self._response = response
        self._error = error
        self.calls: list[dict[str, Any]] = []

    def open(
        self, request: urllib.request.Request, *, timeout: float | None = None
    ) -> _FakeResponse:
        self.calls.append(
            {
                "url": request.full_url,
                "authorization": request.get_header("Authorization"),
                "data": request.data,
                "timeout": timeout,
            }
        )
        if self._error is not None:
            raise self._error
        assert self._response is not None
        return self._response


@pytest.fixture(autouse=True)
def _no_real_socket(monkeypatch: pytest.MonkeyPatch) -> Iterator[None]:
    def blocked(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("the provider HTTP tests must not open a socket")

    monkeypatch.setattr("socket.socket.connect", blocked)
    yield


def _install(monkeypatch: pytest.MonkeyPatch, opener: _FakeOpener) -> _FakeOpener:
    monkeypatch.setattr(urllib.request, "build_opener", lambda *_a, **_k: opener)
    return opener


def _success_body(text: str = "hello", model: str = "gpt-4o-mini") -> bytes:
    return json.dumps({"choices": [{"message": {"content": text}}], "model": model}).encode("utf-8")


# --- success path ---


def test_openai_success_uses_fixed_endpoint_and_bearer(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    opener = _install(monkeypatch, _FakeOpener(response=_FakeResponse(_success_body("hi there"))))
    outcome = openai_compat_chat(
        "openai",
        model="gpt-4o-mini",
        prompt="design a clamp",
        allow_remote=True,
        environ={"OPENAI_API_KEY": OPENAI_KEY},
    )
    assert outcome.text == "hi there"
    assert outcome.model == "gpt-4o-mini"
    assert outcome.provider == "openai"
    assert outcome.offline is False
    assert len(opener.calls) == 1
    call = opener.calls[0]
    assert call["url"] == OPENAI_DEFAULT_ENDPOINT
    assert call["authorization"] == "Bearer " + OPENAI_KEY
    assert call["timeout"] == STUDIO_REQUEST_TIMEOUT_SECONDS
    sent = json.loads(call["data"].decode("utf-8"))
    assert sent["model"] == "gpt-4o-mini"
    assert sent["messages"][0]["role"] == "user"
    assert sent["messages"][0]["content"] == "design a clamp"
    assert sent["stream"] is False


def test_openrouter_success_uses_fixed_endpoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    opener = _install(
        monkeypatch,
        _FakeOpener(response=_FakeResponse(_success_body("reply", model="claude-3-haiku"))),
    )
    outcome = openai_compat_chat(
        "openrouter",
        model="anthropic/claude-3-haiku",
        prompt="hi",
        allow_remote=True,
        environ={"OPENROUTER_API_KEY": OPENROUTER_KEY},
    )
    assert outcome.text == "reply"
    assert outcome.model == "claude-3-haiku"
    call = opener.calls[0]
    assert call["url"] == OPENROUTER_DEFAULT_ENDPOINT
    assert call["authorization"] == "Bearer " + OPENROUTER_KEY


def test_success_body_never_contains_the_key(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    opener = _install(monkeypatch, _FakeOpener(response=_FakeResponse(_success_body("ok"))))
    openai_compat_chat(
        "openai",
        model="gpt-4o-mini",
        prompt="hi",
        allow_remote=True,
        environ={"OPENAI_API_KEY": OPENAI_KEY},
    )
    # The prompt and the key travel in the header, never in the JSON body
    # that a logging layer might capture.
    body_text = opener.calls[0]["data"].decode("utf-8")
    assert OPENAI_KEY not in body_text


# --- consent and credentials ---


def test_remote_refused_before_any_network(monkeypatch: pytest.MonkeyPatch) -> None:
    opener = _install(monkeypatch, _FakeOpener(response=_FakeResponse(_success_body())))
    with pytest.raises(ProviderConfigurationError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=False,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )
    assert opener.calls == []


def test_missing_key_refused_before_any_network(monkeypatch: pytest.MonkeyPatch) -> None:
    opener = _install(monkeypatch, _FakeOpener(response=_FakeResponse(_success_body())))
    with pytest.raises(ProviderMissingCredentialError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={},
        )
    assert opener.calls == []


def test_unknown_provider_has_no_endpoint(monkeypatch: pytest.MonkeyPatch) -> None:
    opener = _install(monkeypatch, _FakeOpener(response=_FakeResponse(_success_body())))
    with pytest.raises(ProviderConfigurationError):
        openai_compat_chat(
            "mystery",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )
    assert opener.calls == []


# --- response parsing failures ---


@pytest.mark.parametrize(
    "body",
    (
        b"not json at all",
        b"[1, 2, 3]",
        b"{}",
        b'{"choices": []}',
        b'{"choices": [{"message": {}}]}',
        b'{"choices": ["not a dict"]}',
    ),
)
def test_malformed_body_raises_protocol_error(monkeypatch: pytest.MonkeyPatch, body: bytes) -> None:
    _install(monkeypatch, _FakeOpener(response=_FakeResponse(body)))
    with pytest.raises(ProviderProtocolError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


def test_observed_model_falls_back_to_requested(monkeypatch: pytest.MonkeyPatch) -> None:
    body = json.dumps({"choices": [{"message": {"content": "ok"}}]}).encode("utf-8")
    _install(monkeypatch, _FakeOpener(response=_FakeResponse(body)))
    outcome = openai_compat_chat(
        "openai",
        model="gpt-4o-mini",
        prompt="hi",
        allow_remote=True,
        environ={"OPENAI_API_KEY": OPENAI_KEY},
    )
    assert outcome.model == "gpt-4o-mini"


# --- response bounds ---


def test_declared_content_length_over_bound_is_refused(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    over = str(MAX_HTTP_RESPONSE_BYTES + 1)
    _install(
        monkeypatch,
        _FakeOpener(response=_FakeResponse(b"x", headers={"Content-Length": over})),
    )
    with pytest.raises(TooLargeResponseError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


def test_streamed_body_over_bound_is_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    oversized = b"a" * (MAX_HTTP_RESPONSE_BYTES + 64)
    _install(monkeypatch, _FakeOpener(response=_FakeResponse(oversized)))
    with pytest.raises(TooLargeResponseError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


# --- transport error mapping ---


def test_http_error_maps_to_protocol_error(monkeypatch: pytest.MonkeyPatch) -> None:
    error = urllib.error.HTTPError(OPENAI_DEFAULT_ENDPOINT, 500, "server error", None, None)
    _install(monkeypatch, _FakeOpener(error=error))
    with pytest.raises(ProviderProtocolError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


def test_socket_timeout_maps_to_timeout_error(monkeypatch: pytest.MonkeyPatch) -> None:
    error = urllib.error.URLError(TimeoutError("timed out"))
    _install(monkeypatch, _FakeOpener(error=error))
    with pytest.raises(ProviderTimeoutError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


def test_builtin_timeout_maps_to_timeout_error(monkeypatch: pytest.MonkeyPatch) -> None:
    _install(monkeypatch, _FakeOpener(error=TimeoutError("slow")))
    with pytest.raises(ProviderTimeoutError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


def test_other_url_error_maps_to_network_error(monkeypatch: pytest.MonkeyPatch) -> None:
    error = urllib.error.URLError(OSError("name resolution failed"))
    _install(monkeypatch, _FakeOpener(error=error))
    with pytest.raises(ProviderNetworkError):
        openai_compat_chat(
            "openai",
            model="gpt-4o-mini",
            prompt="hi",
            allow_remote=True,
            environ={"OPENAI_API_KEY": OPENAI_KEY},
        )


# --- redirect refusal ---


def test_redirect_handler_refuses_every_redirect() -> None:
    handler = NoRedirectHandler()
    with pytest.raises(ProviderNetworkError):
        handler.redirect_request()
