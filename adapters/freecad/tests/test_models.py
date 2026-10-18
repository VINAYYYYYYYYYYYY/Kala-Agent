"""The free router is asked again when a reply is not a design object."""

from __future__ import annotations

import json
import urllib.request
from collections.abc import Mapping
from typing import Literal

import pytest

from kala_agent.contracts import load_construction
from kala_agent_freecad.cad import models
from kala_agent_freecad.errors import FreeCADDependencyError

_PLATE = (
    '{"summary":"A plate.","components":[{"name":"plate","steps":['
    '{"op":"sketch","profile":"rectangle","width":80,"height":40},'
    '{"op":"extrude","length":8}]}],"assembly":[]}'
)


def _reply(
    content: str, *, reasoning: str = "", model: str = "vendor/model:free"
) -> dict[str, object]:
    message: dict[str, object] = {"content": content}
    if reasoning:
        message["reasoning"] = reasoning
    return {"choices": [{"message": message}], "model": model}


def test_markdown_around_a_design_object_is_accepted(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        models,
        "_post",
        lambda *_args, **_kwargs: _reply(f"Here is the design:\n```json\n{_PLATE}\n```"),
    )

    written = models.write_intent("an 80 x 40 x 8 mm plate")

    assert json.loads(written.text)["summary"] == "A plate."
    assert written.model == f"{models.REASONING_MODEL} via vendor/model:free"


def test_a_design_object_in_the_reasoning_text_is_accepted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        models,
        "_post",
        lambda *_args, **_kwargs: _reply("A screw jack has four parts.", reasoning=_PLATE),
    )

    written = models.write_intent("a screwjack")

    assert json.loads(written.text)["components"][0]["name"] == "plate"


def test_prose_is_asked_again_and_the_second_reply_is_used(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    seen: list[object] = []

    def post(
        _url: str,
        payload: Mapping[str, object],
        **_kwargs: object,
    ) -> dict[str, object]:
        seen.append(payload["messages"])
        if len(seen) == 1:
            return _reply("Sure, I can describe a screw jack.", model="vendor/chatty:free")
        return _reply(_PLATE, model="vendor/json:free")

    monkeypatch.setattr(models, "_post", post)

    written = models.write_intent("a screwjack")

    assert len(seen) == 2
    assert json.loads(written.text)["summary"] == "A plate."
    assert written.model == f"{models.REASONING_MODEL} via vendor/json:free"


def test_two_prose_replies_fail_without_a_design(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        models,
        "_post",
        lambda *_args, **_kwargs: _reply("A screw jack is a lifting machine."),
    )

    with pytest.raises(FreeCADDependencyError, match="did not return a design object"):
        models.write_intent("a screwjack")


class _Body:
    def __init__(self, payload: bytes) -> None:
        self._payload = payload

    def read(self, _limit: int) -> bytes:
        return self._payload

    def __enter__(self) -> _Body:
        return self

    def __exit__(self, *_args: object) -> Literal[False]:
        return False


def test_explain_design_returns_the_model_prose(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        models,
        "_post",
        lambda *_args, **_kwargs: _reply("I made a base and a screw with a helix."),
    )

    text = models.explain_design("a screw jack", "{}")

    assert "screw" in text


def test_reasoning_and_decision_use_separate_keys(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[tuple[str, str, dict[str, object]]] = []

    def urlopen(request: urllib.request.Request, timeout: int) -> _Body:
        del timeout
        url = request.full_url
        headers = dict(request.header_items())
        header = str(headers.get("Authorization"))
        raw = request.data
        assert isinstance(raw, bytes)
        payload = json.loads(raw)
        assert isinstance(payload, dict)
        seen.append((url, header, payload))
        if url == models.CHAT_URL:
            return _Body(json.dumps(_reply(_PLATE, model=models.REASONING_MODEL)).encode())
        return _Body(
            json.dumps(
                {
                    "answers": {
                        "plate": {"noul": 0.8, "type": "noul"},
                        "stop": {"noul": 0.1, "type": "noul"},
                    }
                }
            ).encode()
        )

    monkeypatch.setenv("OPENROUTER_API_KEY", "reasoning-key")
    monkeypatch.setenv("OPENROUTER_DECIDE_API_KEY", "decision-key")
    monkeypatch.setattr(urllib.request, "urlopen", urlopen)

    written = models.write_intent("an 80 x 40 x 8 mm plate")
    choice = models.choose_part("A plate is unbuilt.", {"plate": "Build plate.", "stop": "Stop."})

    assert written.model == models.REASONING_MODEL
    assert choice.choice == "plate"
    assert seen[0][0] == models.CHAT_URL
    assert seen[0][1] == "Bearer reasoning-key"
    assert seen[0][2]["model"] == models.REASONING_MODEL
    assert seen[0][2]["reasoning"] == {"effort": "none"}
    assert "response_format" not in seen[0][2]
    assert seen[1][0] == models.DECIDE_URL
    assert seen[1][1] == "Bearer decision-key"
    assert seen[1][2]["model"] == "respan/span-01-lite"
    questions = seen[1][2]["questions"]
    assert isinstance(questions, dict)
    plate = questions["plate"]
    assert isinstance(plate, dict)
    assert plate["type"] == "noul"


def test_choose_revision_picks_the_highest_score(monkeypatch: pytest.MonkeyPatch) -> None:
    def urlopen(request: urllib.request.Request, timeout: int) -> _Body:
        del timeout
        assert request.full_url == models.DECIDE_URL
        headers = dict(request.header_items())
        assert headers.get("Authorization") == "Bearer decision-key"
        raw = request.data
        assert isinstance(raw, bytes)
        payload = json.loads(raw)
        assert isinstance(payload, dict)
        assert payload["model"] == models.DECIDE_MODEL
        questions = payload["questions"]
        assert isinstance(questions, dict)
        assert set(questions) == {"accept", "script_rewrite", "tool_edit"}
        for question in questions.values():
            assert isinstance(question, dict)
            assert question["type"] == "noul"
        return _Body(
            json.dumps(
                {
                    "answers": {
                        "accept": {"noul": 0.2, "type": "noul"},
                        "script_rewrite": {"noul": 0.91, "type": "noul"},
                        "stop": {"noul": 0.99, "type": "noul"},
                        "tool_edit": {"noul": 0.4, "type": "noul"},
                    }
                }
            ).encode()
        )

    monkeypatch.setenv("OPENROUTER_DECIDE_API_KEY", "decision-key")
    monkeypatch.setattr(urllib.request, "urlopen", urlopen)

    choice = models.choose_revision("The plate meets the request.")

    assert choice.choice == "script_rewrite"


def test_write_analysis_parses_a_solve_step(monkeypatch: pytest.MonkeyPatch) -> None:
    script = load_construction(json.loads(_PLATE))
    solved = {
        "steps": [
            {"name": "StructuralSteel", "op": "material"},
            {"face": 1, "op": "fix"},
            {"face": 2, "force_n": 50, "op": "force"},
            {"op": "mesh"},
            {"op": "solve"},
        ]
    }
    seen: list[object] = []

    def post(
        url: str,
        payload: Mapping[str, object],
        **kwargs: object,
    ) -> dict[str, object]:
        assert url == models.CHAT_URL
        assert kwargs["key"] == "reasoning-key"
        messages = payload["messages"]
        assert isinstance(messages, list)
        system = messages[0]
        assert isinstance(system, dict)
        content = system["content"]
        assert isinstance(content, str)
        assert "solve" in content
        seen.append(messages)
        if len(seen) == 1:
            return _reply('{"steps":[{"op":"mesh"}]}')
        return _reply(json.dumps(solved))

    monkeypatch.setenv("OPENROUTER_API_KEY", "reasoning-key")
    monkeypatch.setattr(models, "_post", post)

    parsed = models.write_analysis("an 80 x 40 x 8 mm plate", script)

    assert len(seen) == 2
    assert parsed.steps[-1].op == "solve"
