"""Unit-suite fixtures that keep the subscription CLI routes safe.

The studio probes the operator's own subscription CLIs (``claude``,
``codex``, ``opencode``) whenever it publishes the provider list or the
settings body. In the unit suite that probe would otherwise launch
whatever CLI happens to be installed on the developer's machine, which the
suite must never do.

The fixture below replaces the probe with an honest "not installed"
verdict, so the rows are exactly the ones a host without the CLIs
publishes. The dedicated CLI-provider module opts out: it drives a **fake**
executable in a temporary directory and needs the real probe.
"""

from __future__ import annotations

from collections.abc import Mapping

import pytest

from kala_agent.studio.providers.cli_runtime import CliProbe, CliSpec


def _probe_without_a_cli(spec: CliSpec, environ: Mapping[str, str]) -> CliProbe:
    """Return the verdict of a host that has no subscription CLI."""
    del environ
    return CliProbe(
        id=spec.id,
        installed=False,
        authenticated=False,
        reason=f"the {spec.name} was not found on PATH or at {spec.path_override_env}",
    )


@pytest.fixture(autouse=True)
def no_real_subscription_cli(
    request: pytest.FixtureRequest,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Never start a real subscription CLI from the unit suite.

    A module that sets ``ALLOW_FAKE_CLI_PROBE = True`` opts out: it drives
    a fake executable and needs the real probe.
    """
    if getattr(request.module, "ALLOW_FAKE_CLI_PROBE", False):
        return
    monkeypatch.setattr(
        "kala_agent.studio.providers.cli_runtime.probe",
        _probe_without_a_cli,
    )
