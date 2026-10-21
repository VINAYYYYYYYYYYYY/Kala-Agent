"""Dispatch layer for the studio chat providers.

The dispatch layer is the only entry point the server uses. It picks the
provider implementation, enforces the ``allow_remote`` gate globally, and
publishes the static provider list that the registry surface returns.
"""

from __future__ import annotations

import json
import os
from collections.abc import Callable, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import Final

from kala_agent.studio.design.intent import (
    DESIGN_INTENT_INSTRUCTION,
    DesignIntent,
    DesignIntentParseError,
    design_intent_schema_text,
    design_schema_excerpt,
    parse_design_intent_text,
    strip_design_json_block,
)
from kala_agent.studio.providers import cli_runtime
from kala_agent.studio.providers.base import (
    ChatOutcome,
    ProviderConfigurationError,
    ProviderNetworkError,
    ProviderProtocolError,
    ProviderSubprocessError,
    ProviderTimeoutError,
    ProviderUnavailableError,
    TooLargeResponseError,
)
from kala_agent.studio.providers.claude_cli import SPEC as CLAUDE_SPEC
from kala_agent.studio.providers.claude_cli import chat as claude_cli_chat
from kala_agent.studio.providers.cli_runtime import CliProbe
from kala_agent.studio.providers.codex_cli import SPEC as CODEX_SPEC
from kala_agent.studio.providers.codex_cli import chat as codex_cli_chat
from kala_agent.studio.providers.local import (
    DEFAULT_LOCAL_MODEL,
    detect_design_family,
    local_chat,
    parse_design_prompt,
)
from kala_agent.studio.providers.openai_compat import (
    OPENAI_DEFAULT_ENDPOINT,
    OPENROUTER_DEFAULT_ENDPOINT,
    STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
    fixed_endpoints,
    openai_compat_chat,
)
from kala_agent.studio.providers.opencode_cli import SPEC as OPENCODE_SPEC
from kala_agent.studio.providers.opencode_cli import chat as opencode_cli_chat
from kala_agent.studio.schemas import ProviderRecord

STUDIO_PROVIDER_TIMEOUT_SECONDS: Final[float] = 30.0
STUDIO_RESPONSE_MAX_BYTES: Final[int] = 4096

#: The wall deadline of one CAD build, in seconds.
#:
#: The FreeCAD provider's default deadline
#: (``kala_agent.studio.design.providers.freecad.DEFAULT_TIMEOUT_SECONDS``,
#: 1 800 s) covers a measured 500-step build (17-21 min); the OpenCascade
#: compile budget (60 s) fits inside it. It is restated here as an integer so
#: ``/health`` can publish the turn budget in every build; a unit test pins it
#: to the provider constant.
DESIGN_BUILD_TIMEOUT_SECONDS: Final[int] = 1_800
#: A design turn builds at most twice: the first attempt and one repair build.
DESIGN_TURN_BUILD_ATTEMPTS: Final[int] = 2


def design_turn_budget_seconds() -> int:
    """Return the backend's own worst-case design-turn wall time, in seconds.

    Each build attempt is preceded by one model call, so the worst case is
    ``attempts * (model call + build)``: 2 * (180 + 1 800) = 3 960 s (66 min).
    ``/health`` publishes this value; the Studio client derives its own
    deadline from it instead of pinning a number the design budget can
    outgrow. The Rust service publishes the same value and a differential
    case compares the two.
    """
    return int(
        DESIGN_TURN_BUILD_ATTEMPTS
        * (STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS + DESIGN_BUILD_TIMEOUT_SECONDS)
    )

#: The provider ids the Studio publishes, in the published order.
#:
#: ``codex`` is the historical id of the Codex CLI route. It stays in the
#: list so an old saved preference still resolves; it names the same CLI as
#: ``codex_cli``.
PROVIDER_IDS: Final[tuple[str, ...]] = (
    "local",
    "openai",
    "openrouter",
    "codex",
    "claude_code",
    "codex_cli",
    "opencode_cli",
)

#: The source the Studio reports for a subscription CLI's credential.
#:
#: The credential is the operator's own CLI login. Kala never sees it, never
#: stores it and never forwards it.
SUBSCRIPTION_SOURCE: Final[str] = "subscription (CLI)"

#: The subscription provider ids that run an operator-owned CLI.
SUBSCRIPTION_PROVIDER_IDS: Final[tuple[str, ...]] = (
    "codex",
    "claude_code",
    "codex_cli",
    "opencode_cli",
)

#: The provider call one remote turn makes.
#:
#: The call receives the prompt and the budget of that one call and answers
#: with the provider's reply. A budget of ``None`` means "the provider's own
#: default".
ChatCaller = Callable[[str, float | None], ChatOutcome]

#: The largest previous JSON block the validation-repair prompt carries.
#:
#: Sized from the measured maximum design: a 500-step canonical script is
#: 90 497 characters with worst-case values (81 366 for a realistic 100-hole
#: plate), and a pretty-printed maximum is 171 546. 200 000 holds every one of
#: those whole, so a repair always sees its own complete JSON, while the reply
#: itself stays bounded by the 1 MB transport limit.
MAX_REPAIR_PREVIOUS_CHARS: Final[int] = 200_000
#: The largest validation error the repair prompt and the honest text carry.
MAX_REPAIR_ERROR_CHARS: Final[int] = 512

#: The largest compact script the build-repair prompt carries.
#:
#: A 500-step script is 90 497 characters in canonical JSON with worst-case
#: values and 81 366 with realistic ones; 200 000 holds either whole, so the
#: build repair never reads a script cut mid-object.
MAX_BUILD_REPAIR_SCRIPT_CHARS: Final[int] = 200_000
#: The largest build feedback the build-repair prompt carries.
MAX_BUILD_REPAIR_FEEDBACK_CHARS: Final[int] = 2_000
#: The largest schema excerpt the build-repair prompt carries.
MAX_BUILD_REPAIR_SCHEMA_CHARS: Final[int] = 700
#: The largest length of the build-repair prompt without the script.
#:
#: The script is bounded separately by ``MAX_BUILD_REPAIR_SCRIPT_CHARS``, and
#: the user's own request is appended unchanged and is not part of this bound.
#:
#: The user's own request is appended unchanged and is not part of this bound,
#: exactly like in the first design prompt.
MAX_BUILD_REPAIR_PROMPT_CHARS: Final[int] = 9_000


def _provider_error_code(error: BaseException) -> str:
    """Return the sanitized envelope code of one provider failure.

    This is the same mapping the route answers an envelope with, so the honest
    reason a failed repair call states names the code the operator would have
    seen.
    """
    if isinstance(error, ProviderTimeoutError):
        return "provider_timeout"
    if isinstance(error, ProviderNetworkError):
        return "provider_network_error"
    if isinstance(error, ProviderProtocolError):
        return "provider_protocol_error"
    if isinstance(error, ProviderSubprocessError):
        return "provider_subprocess_error"
    if isinstance(error, ProviderUnavailableError):
        return "provider_unavailable"
    if isinstance(error, ProviderConfigurationError):
        return str(getattr(error, "code", "provider_not_configured"))
    # An oversized body and every other unclassified failure: the route has no
    # envelope code for them, and the native product maps the same failures to
    # its internal error.
    return "internal_error"


@dataclass(frozen=True, slots=True)
class BuildFeedback:
    """The concrete facts one failed build hands back to the model.

    Every value here comes from the worker's own report, never from a guess:
    the script the model sent, the check rows that failed, the provider
    warnings, and the builder refusal.
    """

    script: str = ""
    failed_checks: tuple[str, ...] = ()
    warnings: tuple[str, ...] = ()
    refusal: str | None = None

    def text(self) -> str:
        """Return the bounded feedback body the repair prompt carries."""
        body = f"the script you sent:\n{self.script}\n"
        if self.refusal is not None:
            body += f"\nthe builder refused the script:\n{self.refusal}\n"
        if self.failed_checks or self.warnings:
            body += "\nthe build reported:\n"
            for check in self.failed_checks:
                body += f"- {check}\n"
            for warning in self.warnings:
                body += f"- warning: {warning}\n"
        return body.strip()

    def reason(self) -> str:
        """Return the one short sentence that says why the attempt failed."""
        if self.refusal is not None:
            return f"the builder refused the script: {self.refusal}"
        checks = " and ".join(self.failed_checks)
        warning = ""
        if self.warnings:
            warning = f"; the provider warned: {self.warnings[0]}"
        if not checks:
            if not warning:
                return "the build did not produce a verified part"
            return f"the build was not verified{warning}"
        return f"the build failed {checks}{warning}"


class StudioChatTurn(ChatOutcome):
    """A provider reply plus any design intent the turn produced.

    The extra fields are metadata for the server, not part of the HTTP
    response. ``design_intent`` is present only when the prompt was parsed or
    validated into a complete intent; ``design_missing`` names the values the
    local helper refused to guess. ``design_error`` records an honest parse
    or repair failure. ``design_repair_spent`` records that the turn already
    spent its one repair call on the validation, so the build may not spend
    another one. ``design_repair_failure`` records that the repair CALL
    itself failed, naming the envelope code, so the server keeps the first
    build instead of losing it to a provider timeout.
    """

    def __init__(
        self,
        *,
        text: str,
        provider: str,
        model: str | None,
        offline: bool,
        design_intent: DesignIntent | None = None,
        design_missing: tuple[str, ...] = (),
        design_notes: tuple[str, ...] = (),
        design_family: str | None = None,
        design_error: str | None = None,
        design_repair_spent: bool = False,
        design_repair_failure: str | None = None,
        cad_provider: str | None = None,
    ) -> None:
        super().__init__(text=text, provider=provider, model=model, offline=offline)
        self.design_intent = design_intent
        self.design_missing = design_missing
        self.design_notes = design_notes
        self.design_family = design_family
        self.design_error = design_error
        self.design_repair_spent = design_repair_spent
        self.design_repair_failure = design_repair_failure
        self.cad_provider = cad_provider


def _design_prompt(prompt: str) -> str:
    schema = design_intent_schema_text()
    return (
        f"{DESIGN_INTENT_INSTRUCTION}\n\n"
        f"DesignIntent JSON Schema:\n{schema}\n\n"
        f"User request:\n{prompt}"
    )


def _repair_prompt(prompt: str, error: str, previous: str) -> str:
    """Return the prompt that repairs a reply that is not a valid DesignIntent.

    Every model call is stateless. The repair call therefore carries the same
    ``DESIGN_INTENT_INSTRUCTION`` and the same full DesignIntent schema the
    first call carried, the JSON block the model itself sent (bounded), the
    exact validation error and a bounded schema excerpt for the field that
    failed. Without its own previous JSON the repair re-authors the design
    blind: it invents a different shape and loses the values it had.
    """
    error = error[:MAX_REPAIR_ERROR_CHARS]
    excerpt = design_schema_excerpt(error)
    schema_section = ""
    if excerpt is not None:
        location = _error_location(error) or "the failing field"
        schema_section = f"\n\nSchema for {location}:\n{excerpt}"
    return (
        f"{DESIGN_INTENT_INSTRUCTION}\n\n"
        f"DesignIntent JSON Schema:\n{design_intent_schema_text()}"
        f"{_previous_design_json_section(previous)}\n\n"
        "Your previous reply could not be validated as a DesignIntent: "
        f"{error}"
        f"{schema_section}\n\n"
        "Return exactly one fenced ```json block containing a corrected "
        "DesignIntent for the same request. Keep every value the user stated "
        "and do not invent a value the user did not state. If a required "
        "value is missing, reply in prose with the missing values instead.\n\n"
        f"User request:\n{prompt}"
    )


def _previous_design_json_section(previous: str) -> str:
    """Return the section the repair prompt carries about the previous reply.

    The model must see the JSON it sent: without it the repair re-authors the
    design blind. The block is extracted from the reply and bounded. A reply
    whose JSON is not a balanced object still shows its own text, so the model
    sees exactly what it wrote.
    """
    block = _extract_design_json_block(previous)
    if block is None:
        header = "Your previous reply:"
        body = previous.strip()
    else:
        header = "The JSON block you sent:"
        body = block
    body = body[:MAX_REPAIR_PREVIOUS_CHARS]
    if not body:
        return ""
    return f"\n\n{header}\n{body}"


def _extract_design_json_block(text: str) -> str | None:
    """Return the JSON block the model sent in its previous reply.

    The block is taken from the first fenced block that carries an object, and
    from the text outside the fences when no fence carries one. A bare object
    in prose is found the same way.
    """
    fences = _fence_spans(text)
    for _outer_start, _outer_end, content_start, content_end in fences:
        content = text[content_start:content_end]
        span = _first_object_span(content)
        if span is not None:
            return content[span[0] : span[1]]
    parts: list[str] = []
    last = 0
    for outer_start, outer_end, _content_start, _content_end in fences:
        parts.append(text[last:outer_start])
        last = outer_end
    parts.append(text[last:])
    outside = "".join(parts)
    span = _first_object_span(outside)
    if span is None:
        return None
    return outside[span[0] : span[1]]


#: The characters a fence's language tag may carry.
_FENCE_LANGUAGE: Final[str] = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-"


def _fence_spans(text: str) -> tuple[tuple[int, int, int, int], ...]:
    """Return ``(outer_start, outer_end, content_start, content_end)`` per fence.

    The language tag is skipped, so an unlabelled fence and a ```json fence
    behave the same. An unclosed fence runs to the end of the reply.
    """
    spans: list[tuple[int, int, int, int]] = []
    length = len(text)
    index = 0
    while True:
        opening = text.find("```", index)
        if opening < 0:
            break
        cursor = opening + 3
        while cursor < length and text[cursor] in " \t":
            cursor += 1
        while cursor < length and text[cursor] in _FENCE_LANGUAGE:
            cursor += 1
        while cursor < length and text[cursor] in " \t":
            cursor += 1
        if text.startswith("\r\n", cursor):
            cursor += 2
        elif cursor < length and text[cursor] == "\n":
            cursor += 1
        closing = text.find("```", cursor)
        if closing < 0:
            spans.append((opening, length, cursor, length))
            break
        spans.append((opening, closing + 3, cursor, closing))
        index = closing + 3
    return tuple(spans)


def _first_object_span(text: str) -> tuple[int, int] | None:
    """Return the span of the first balanced ``{...}`` object, strings respected."""
    start = 0
    depth = 0
    in_string = False
    escaped = False
    for index, character in enumerate(text):
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
            continue
        if character == '"':
            if depth > 0:
                in_string = True
        elif character == "{":
            if depth == 0:
                start = index
            depth += 1
        elif character == "}":
            if depth > 0:
                depth -= 1
                if depth == 0:
                    return (start, index + 1)
    return None


def _error_location(error: str) -> str:
    """Return the field path an error names, when it is a dotted identifier path.

    An error whose head is a whole sentence names no field: the prompt then
    says "the failing field", exactly like the native product.
    """
    head = error.split(": ", 1)[0].strip()
    if not head:
        return ""
    for token in head.split("."):
        if not token or not all(character.isalnum() or character == "_" for character in token):
            return ""
    return head


def _validation_repair_failed_reason(first_error: str, second_error: str) -> str:
    """Return the reason a turn that spent its repair on the validation built nothing.

    **Both** errors are named: the first reply's own error and the repair's
    error. A user who reads only the last line never learns what went wrong
    the first time, and the first failure is the one that explains why the
    repair was needed at all. Each error is bounded.
    """
    return (
        "the first reply was not a valid design "
        f"({first_error[:MAX_REPAIR_ERROR_CHARS]}), and the reply after one repair "
        "attempt was not a valid design either "
        f"({second_error[:MAX_REPAIR_ERROR_CHARS]})."
    )


_NO_DESIGN_PROSE: Final[str] = "Design intent received."


def _build_repair_schema_excerpt(schema_text: str) -> str:
    """Return the bounded schema excerpt the build-repair prompt carries.

    The whole DesignIntent schema is about 11 KB; the build repair only needs
    the shape of the script, so the excerpt is the ``script`` property of the
    root schema, and the root summary when that property cannot be read.
    """
    try:
        schema = json.loads(schema_text)
    except (TypeError, ValueError):
        schema = {}
    if not isinstance(schema, dict):
        schema = {}
    properties = schema.get("properties")
    node = properties.get("script") if isinstance(properties, dict) else None
    if node is None:
        node = {
            "type": "object",
            "required": list(schema.get("required", [])),
            "properties": {
                name: {} for name in (properties if isinstance(properties, dict) else {})
            },
        }
    return json.dumps(node, indent=2, sort_keys=True, ensure_ascii=False)[
        :MAX_BUILD_REPAIR_SCHEMA_CHARS
    ]


def _build_repair_prompt(prompt: str, feedback: str, schema_text: str) -> str:
    """Return the prompt that repairs a script the builder refused or failed.

    Every model call is stateless, so the repair is a fresh call and the model
    no longer sees the supported subset, the plane rule or the worked examples
    it read the first time. The prompt therefore carries the same
    ``DESIGN_INTENT_INSTRUCTION`` the first call carried, unchanged, followed
    by the user's original request, the script the model sent (compact JSON,
    bounded) and the concrete build feedback: the failed check rows, the
    provider warnings, or the builder's refusal.
    """
    return (
        f"{DESIGN_INTENT_INSTRUCTION}\n\n"
        "Your previous reply was built, but the build was rejected: "
        f"{feedback[:MAX_BUILD_REPAIR_FEEDBACK_CHARS]}"
        "\n\n"
        "Return exactly one fenced ```json block containing a corrected "
        "DesignIntent for the same request. Fix the script so that every cut "
        "removes material and the part matches the request, and keep every "
        "dimension the user stated: do not invent a value the user did not "
        "state and do not change a value the user did state. If a required "
        "value is missing, reply in prose with the missing values instead."
        f"\n\nSchema for script:\n{_build_repair_schema_excerpt(schema_text)}"
        f"\n\nUser request:\n{prompt}"
    )


def build_repair_turn(
    provider: str,
    *,
    model: str,
    prompt: str,
    feedback: str,
    allow_remote: bool,
    environ: Mapping[str, str],
    call: ChatCaller | None = None,
) -> StudioChatTurn:
    """Spend the turn's one repair call on a failed build.

    The reply is validated like any other design reply: a reply that is not a
    valid DesignIntent comes back with no ``design_intent`` and an honest
    ``design_error``, and the server keeps the state of the first attempt.

    A repair CALL that fails is not a turn failure either. The first design
    was built and journalled already, so a provider that times out on the
    repair call must not throw it away: the turn comes back with
    ``design_repair_failure`` naming the envelope code, and the server keeps
    the first design and says why the repair did not happen.
    """
    if call is None:
        call = provider_call(
            provider,
            model=model,
            allow_remote=allow_remote,
            environ=environ,
        )
    try:
        reply = call(
            _build_repair_prompt(prompt, feedback, design_intent_schema_text()),
            STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
        )
    except (
        ProviderConfigurationError,
        ProviderNetworkError,
        ProviderProtocolError,
        ProviderSubprocessError,
        ProviderTimeoutError,
        TooLargeResponseError,
    ) as error:
        # ``ProviderUnavailableError`` is a ``ProviderConfigurationError``, so
        # it is caught here too and keeps its own envelope code.
        return StudioChatTurn(
            text="",
            provider=provider,
            model=model,
            offline=False,
            design_repair_failure=f"the repair attempt failed: {_provider_error_code(error)}.",
            design_repair_spent=True,
        )
    text = _display_text(reply.reply)
    if "{" not in reply.reply:
        return StudioChatTurn(
            text=text,
            provider=provider,
            model=reply.model or model,
            offline=False,
            design_error="the repair attempt did not include a design.",
            design_repair_spent=True,
        )
    try:
        intent = parse_design_intent_text(reply.reply)
    except DesignIntentParseError as error:
        return StudioChatTurn(
            text=text,
            provider=provider,
            model=reply.model or model,
            offline=False,
            design_error=str(error),
            design_repair_spent=True,
        )
    return StudioChatTurn(
        text=text,
        provider=provider,
        model=reply.model or model,
        offline=False,
        design_intent=intent,
        design_repair_spent=True,
    )


def _compact_script(intent: DesignIntent) -> str:
    """Return the compact JSON of the script the model sent."""
    script = getattr(intent, "script", None)
    if script is None:
        return ""
    payload = script.model_dump(mode="json", exclude_none=True)
    return json.dumps(payload, separators=(",", ":"))[:MAX_BUILD_REPAIR_SCRIPT_CHARS]


def _compact_script_text(script: object) -> str:
    """Return the compact JSON of a built design's script."""
    if not isinstance(script, dict):
        return ""
    return json.dumps(script, separators=(",", ":"))[:MAX_BUILD_REPAIR_SCRIPT_CHARS]


def _render_check_value(value: object) -> str:
    """Render one check value the way the repair prompt states it."""
    if value is None:
        return "not reported"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return str(int(value)) if value.is_integer() else str(value)
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_render_check_value(item) for item in value) + "]"
    return str(value)


def build_feedback(
    intent: DesignIntent,
    *,
    verified: bool = True,
    script: object = None,
    checks: Sequence[object] = (),
    warnings: Sequence[str] = (),
    refusal: str | None = None,
) -> BuildFeedback | None:
    """Return the feedback one failed build produces, or ``None``.

    A design that built and verified has no feedback: nothing is wrong. A
    design that built and failed a check reports the failed rows and the
    provider warnings. A builder refusal reports the refusal itself.
    """
    if refusal is not None:
        return BuildFeedback(
            script=_compact_script_text(script) or _compact_script(intent),
            refusal=refusal,
        )
    if verified:
        return None
    failed = [check for check in checks if getattr(check, "passed", True) is not True]
    if not failed and not warnings:
        return None
    rows = tuple(
        "the check '{id}' (expected {expected}, measured {measured})".format(
            id=getattr(check, "id", ""),
            expected=_render_check_value(getattr(check, "expected", None)),
            measured=_render_check_value(getattr(check, "measured", None)),
        )
        for check in failed
    )
    return BuildFeedback(
        script=_compact_script_text(script) or _compact_script(intent),
        failed_checks=rows,
        warnings=tuple(warnings),
    )


def _no_build_text(prose: str, reason: str) -> str:
    if prose == _NO_DESIGN_PROSE:
        return f"No part was built: {reason}"
    return f"{prose} No part was built: {reason}"


#: The display bound of one chat reply.
#:
#: The design JSON is read from the full reply; only the displayed prose keeps
#: this bound, exactly like ``openai_compat.MAX_RESPONSE_TEXT_LENGTH``.
MAX_DISPLAY_TEXT_LENGTH: Final[int] = 4_096


def _display_text(reply: str) -> str:
    """Return the displayed prose: the design JSON block stripped and bounded."""
    return strip_design_json_block(reply)[:MAX_DISPLAY_TEXT_LENGTH]


def probe_subscription_clis(environ: Mapping[str, str] | None = None) -> tuple[CliProbe, ...]:
    """Probe the three subscription CLIs on this host.

    The probe is cheap and cached: it asks each CLI for its version and for
    its own authentication status, and it never reads a credential file.
    Only the exit status of the status command is used, so no account name,
    address or token can reach a response.

    The three probes run concurrently, exactly like the native product, so
    a slow authentication check does not add up.

    This form waits for every CLI. The HTTP routes use
    :func:`probe_subscription_clis_cached` instead, which never waits for a
    CLI.
    """
    if environ is None:
        environ = os.environ
    specs = (CLAUDE_SPEC, CODEX_SPEC, OPENCODE_SPEC)
    with ThreadPoolExecutor(max_workers=len(specs)) as pool:
        futures = [pool.submit(cli_runtime.probe, spec, environ) for spec in specs]
        return tuple(future.result() for future in futures)


def probe_subscription_clis_cached(
    environ: Mapping[str, str] | None = None,
) -> tuple[CliProbe, ...]:
    """Return the three subscription verdicts, answered from the cache.

    No route may wait for a CLI: a cached (even stale) verdict is returned at
    once and refreshed in the background, and a first probe that has not
    finished inside the route budget is reported as ``checking``.
    """
    if environ is None:
        environ = os.environ
    return cli_runtime.probe_subscription_clis_cached(
        (CLAUDE_SPEC, CODEX_SPEC, OPENCODE_SPEC),
        environ,
    )


def _subscription_notes(name: str, binary: str, extra: str) -> str:
    """Return the notes of one subscription-CLI provider."""
    return (
        f"{name}. Runs the operator's own `{binary}` CLI as a sandboxed "
        "subprocess: no shell, no tools, a private working directory, a "
        "sanitized environment and a bounded run. "
        f"{extra} Kala never sees or stores the CLI's credential. Requires "
        "allow_remote=true at request time."
    )


def _probe_for(clis: Sequence[CliProbe], provider_id: str) -> CliProbe | None:
    """Return the probe verdict of one subscription CLI, when there is one."""
    for probe in clis:
        if probe.id == provider_id:
            return probe
    return None


def _subscription_record(
    provider_id: str,
    name: str,
    notes: str,
    probe: CliProbe | None,
) -> ProviderRecord:
    """Return one subscription-CLI provider row.

    The row is ``configured`` only when the CLI is installed **and** reports
    that it is signed in. The row is never a guess: a missing CLI carries
    the honest reason, and a probe that is still running is reported as
    unknown (``available=None`` with the "checking" reason), never as
    unavailable.
    """
    usable = probe is not None and probe.usable()
    checking = probe is not None and probe.checking
    return ProviderRecord(
        id=provider_id,
        name=name,
        configured=usable,
        notes=notes,
        available=None if checking else usable,
        reason=(
            cli_runtime.PROBE_CHECKING_REASON
            if checking
            else (None if usable or probe is None else probe.reason)
        ),
        source=SUBSCRIPTION_SOURCE,
    )


def list_provider_records(
    environ: Mapping[str, str] | None = None,
    clis: Sequence[CliProbe] | None = None,
) -> tuple[ProviderRecord, ...]:
    """Return the sanitised provider list for ``/api/studio/providers``.

    ``configured`` is reported by inspecting the controlled environment
    view the caller passes; the studio never leaks the keys themselves.

    The subscription rows carry the availability probe's verdict: a CLI
    that is installed and signed in is ``configured``; one that is missing
    or not signed in is reported as unavailable with the honest reason. The
    probe never reads a credential file, and the rows never disclose the
    local CLI installation path.
    """
    if environ is None:
        environ = os.environ
    if clis is None:
        clis = probe_subscription_clis(environ)
    openai_configured = bool(_peek_credential("openai", environ))
    openrouter_configured = bool(_peek_credential("openrouter", environ))
    codex_probe = _probe_for(clis, CODEX_SPEC.id)
    codex_notes = _subscription_notes(
        "Codex CLI (ChatGPT subscription)",
        "codex",
        "The turn runs in a read-only sandbox with no session transcript and "
        "without the operator's own Codex configuration.",
    )
    claude_notes = _subscription_notes(
        "Claude Code CLI (Anthropic subscription)",
        "claude",
        "The turn runs with every built-in tool removed and with the "
        "operator's hooks, plugins, skills and MCP servers switched off.",
    )
    opencode_notes = _subscription_notes(
        "OpenCode CLI (subscription)",
        "opencode",
        "The turn runs as an agent that may not call any tool, in a private working directory.",
    )

    return (
        ProviderRecord(
            id="local",
            name="Local Deterministic Helper",
            configured=True,
            notes=(
                "Offline deterministic mechanical-engineering helper. Extracts "
                "dimensions and returns nominal analytic volume/surface-area "
                "estimates, and documents the built-in OpenCascade (OCP) "
                "STEP/STL estimate, neutral CAD/CAE handoff, and standalone "
                "build workflow. Opens no network connection and runs no solver."
            ),
            available=None,
            reason=None,
            source=None,
        ),
        ProviderRecord(
            id="openai",
            name="OpenAI (Chat Completions)",
            configured=openai_configured,
            notes=(
                "API-key route; uses the standard Chat Completions endpoint at "
                + OPENAI_DEFAULT_ENDPOINT
                + ". Requires allow_remote=true at request time."
            ),
            available=None,
            reason=None,
            source=None,
        ),
        ProviderRecord(
            id="openrouter",
            name="OpenRouter (Chat Completions)",
            configured=openrouter_configured,
            notes=(
                "API-key route; uses the public OpenRouter endpoint at "
                + OPENROUTER_DEFAULT_ENDPOINT
                + ". Requires allow_remote=true at request time."
            ),
            available=None,
            reason=None,
            source=None,
        ),
        _subscription_record(
            "codex",
            "Codex CLI (ChatGPT subscription)",
            codex_notes,
            codex_probe,
        ),
        _subscription_record(
            CLAUDE_SPEC.id,
            "Claude Code CLI (Anthropic subscription)",
            claude_notes,
            _probe_for(clis, CLAUDE_SPEC.id),
        ),
        _subscription_record(
            CODEX_SPEC.id,
            "Codex CLI (ChatGPT subscription)",
            codex_notes,
            codex_probe,
        ),
        _subscription_record(
            OPENCODE_SPEC.id,
            "OpenCode CLI (subscription)",
            opencode_notes,
            _probe_for(clis, OPENCODE_SPEC.id),
        ),
    )


def _peek_credential(provider: str, environ: Mapping[str, str]) -> str:
    candidates = {
        "openai": ("OPENAI_API_KEY",),
        "openrouter": ("OPENROUTER_API_KEY",),
    }
    for name in candidates.get(provider, ()):
        value = environ.get(name)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""


def chat(
    *,
    provider: str,
    model: str | None,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str] | None = None,
    cad_provider: str | None = None,
) -> ChatOutcome:
    """Dispatch a chat request to the named provider.

    ``allow_remote`` is honoured globally: ``local`` ignores it; ``openai``
    and ``openrouter`` require it for any non-trivial use; the four
    subscription ids require it too, because they invoke an out-of-process
    CLI that talks to the operator's vendor. A caller that asks for a
    provider the registry does not publish is refused with
    :class:`ProviderConfigurationError`.

    Design-intent hook: the local helper's strict parser and the remote
    providers' JSON-Schema prompt both return a :class:`StudioChatTurn` so
    the server can compile the intent without the provider layer ever
    touching OCP. ``allow_remote`` semantics are unchanged.
    """
    if environ is None:
        environ = os.environ
    if provider == "local":
        parsed = parse_design_prompt(prompt)
        outcome = local_chat(prompt, model=model or DEFAULT_LOCAL_MODEL)
        if parsed is None:
            return StudioChatTurn(
                text=outcome.text,
                provider=outcome.provider,
                model=outcome.model,
                offline=outcome.offline,
                cad_provider=cad_provider,
            )
        return StudioChatTurn(
            text=outcome.text,
            provider=outcome.provider,
            model=outcome.model,
            offline=outcome.offline,
            design_intent=parsed.intent,
            design_missing=parsed.missing,
            design_notes=parsed.notes,
            design_family=parsed.family,
            cad_provider=cad_provider,
        )
    if provider == "openai":
        if not model:
            raise ProviderConfigurationError(
                "openai provider requires a model id (e.g. 'gpt-4o-mini')"
            )
        return _remote_turn(
            "openai",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
            cad_provider=cad_provider,
        )
    if provider == "openrouter":
        if not model:
            raise ProviderConfigurationError("openrouter provider requires a model id")
        return _remote_turn(
            "openrouter",
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
            cad_provider=cad_provider,
        )
    if provider in SUBSCRIPTION_PROVIDER_IDS:
        return _subscription_turn(
            provider,
            model=model,
            prompt=prompt,
            allow_remote=allow_remote,
            environ=environ,
            cad_provider=cad_provider,
        )
    raise ProviderConfigurationError(f"unknown provider: {provider!r}")


def _subscription_turn(
    provider: str,
    *,
    model: str | None,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str],
    cad_provider: str | None = None,
) -> StudioChatTurn:
    """Run one chat turn through the operator's own subscription CLI.

    The route is remote egress (ADR-0001), so it needs the same
    ``allow_remote`` consent the two API-key routes need. The refusal is
    refused before the CLI is started.
    """
    if not allow_remote:
        raise ProviderConfigurationError(cli_runtime.consent_message(provider))
    return _remote_turn(
        provider,
        model=model,
        prompt=prompt,
        allow_remote=allow_remote,
        environ=environ,
        cad_provider=cad_provider,
        call=provider_call(
            provider,
            model=model,
            allow_remote=allow_remote,
            environ=environ,
        ),
    )


#: The chat function of every subscription provider id.
#:
#: The historical ``codex`` id and the current ``codex_cli`` id name the
#: same CLI, so both run the same function.
SubscriptionRunner = Callable[[str | None, str, Mapping[str, str]], ChatOutcome]

_SUBSCRIPTION_RUNNERS: Final[dict[str, SubscriptionRunner]] = {
    "codex": codex_cli_chat,
    "codex_cli": codex_cli_chat,
    "claude_code": claude_cli_chat,
    "opencode_cli": opencode_cli_chat,
}


def provider_call(
    provider: str,
    *,
    model: str | None,
    allow_remote: bool,
    environ: Mapping[str, str],
) -> ChatCaller:
    """Return the provider call one turn makes.

    The two API-key routes use the fixed OpenAI-compatible endpoint; the
    four subscription routes run the operator's own CLI. Both keep the same
    signature, so the design-intent flow does not care which one answered.
    """
    if provider in SUBSCRIPTION_PROVIDER_IDS:
        runner = _SUBSCRIPTION_RUNNERS[provider]

        def cli_call(text: str, timeout: float | None) -> ChatOutcome:
            del timeout
            return runner(model, text, environ)

        return cli_call

    def api_call(text: str, timeout: float | None) -> ChatOutcome:
        if timeout is None:
            # A plain chat leaves the provider's own default budget alone.
            return openai_compat_chat(
                provider,
                model=model or "",
                prompt=text,
                allow_remote=allow_remote,
                environ=environ,
            )
        return openai_compat_chat(
            provider,
            model=model or "",
            prompt=text,
            allow_remote=allow_remote,
            environ=environ,
            timeout=timeout,
        )

    return api_call


def _remote_turn(
    provider: str,
    *,
    model: str | None,
    prompt: str,
    allow_remote: bool,
    environ: Mapping[str, str],
    cad_provider: str | None = None,
    call: ChatCaller | None = None,
) -> StudioChatTurn:
    """Run one remote turn, with the design prompt when the turn needs one.

    ``call`` is the provider call the turn makes. The default is the
    provider's own call; the subscription routes pass their own CLI call,
    which keeps the same design-intent contract.
    """
    if call is None:
        call = provider_call(
            provider,
            model=model,
            allow_remote=allow_remote,
            environ=environ,
        )

    family = detect_design_family(prompt)
    if family is None and cad_provider is None:
        outcome = call(prompt, None)
        return StudioChatTurn(
            text=outcome.text,
            provider=outcome.provider,
            model=outcome.model,
            offline=outcome.offline,
            cad_provider=cad_provider,
        )

    first = call(_design_prompt(prompt), STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS)
    if "{" not in first.reply:
        # A question about the values the model still needs is a legitimate
        # design answer: never spend a repair turn or invent the values.
        return StudioChatTurn(
            text=_no_build_text(
                _display_text(first.reply),
                "the reply did not include a design.",
            ),
            provider=provider,
            model=first.model or model,
            offline=False,
            design_family=family,
            cad_provider=cad_provider,
        )
    try:
        intent = parse_design_intent_text(first.reply)
    except DesignIntentParseError as first_error:
        # The repair call is stateless: it carries the model's own previous
        # JSON, so it corrects its own answer instead of re-authoring one.
        second = call(
            _repair_prompt(prompt, str(first_error), first.reply),
            STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS,
        )
        if "{" not in second.reply:
            return StudioChatTurn(
                text=_no_build_text(
                    _display_text(second.reply),
                    "the reply asks for values instead of including a design.",
                ),
                provider=provider,
                model=second.model or first.model or model,
                offline=False,
                design_family=family,
                design_repair_spent=True,
                cad_provider=cad_provider,
            )
        try:
            intent = parse_design_intent_text(second.reply)
        except DesignIntentParseError as second_error:
            prose = _display_text(second.reply)
            return StudioChatTurn(
                text=_no_build_text(
                    prose,
                    _validation_repair_failed_reason(str(first_error), str(second_error)),
                ),
                provider=provider,
                model=second.model or first.model or model,
                offline=False,
                design_family=family,
                design_error=str(second_error),
                design_repair_spent=True,
                cad_provider=cad_provider,
            )
        return StudioChatTurn(
            text=_display_text(second.reply),
            provider=provider,
            model=second.model or first.model or model,
            offline=False,
            design_intent=intent,
            design_family=family,
            design_repair_spent=True,
            cad_provider=cad_provider,
        )
    return StudioChatTurn(
        text=_display_text(first.reply),
        provider=provider,
        model=first.model or model,
        offline=False,
        design_intent=intent,
        design_family=family,
        cad_provider=cad_provider,
    )


__all__ = (
    "DESIGN_BUILD_TIMEOUT_SECONDS",
    "DESIGN_TURN_BUILD_ATTEMPTS",
    "MAX_BUILD_REPAIR_FEEDBACK_CHARS",
    "MAX_BUILD_REPAIR_PROMPT_CHARS",
    "MAX_BUILD_REPAIR_SCHEMA_CHARS",
    "MAX_BUILD_REPAIR_SCRIPT_CHARS",
    "MAX_DISPLAY_TEXT_LENGTH",
    "MAX_REPAIR_ERROR_CHARS",
    "MAX_REPAIR_PREVIOUS_CHARS",
    "PROVIDER_IDS",
    "STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS",
    "STUDIO_PROVIDER_TIMEOUT_SECONDS",
    "STUDIO_RESPONSE_MAX_BYTES",
    "SUBSCRIPTION_PROVIDER_IDS",
    "SUBSCRIPTION_SOURCE",
    "BuildFeedback",
    "StudioChatTurn",
    "build_feedback",
    "build_repair_turn",
    "chat",
    "design_turn_budget_seconds",
    "fixed_endpoints",
    "list_provider_records",
    "probe_subscription_clis",
)
