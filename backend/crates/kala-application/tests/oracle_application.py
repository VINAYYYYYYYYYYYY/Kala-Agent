"""The live Python oracle for the kala-application differential.

The oracle imports the repository's own ``kala_agent.application`` and drives
the same operation script the Rust test drives. It reads a scenario name, a
data directory and an optional mode from argv and writes one canonical JSON
document to stdout.

Test-only: the Rust crate never invokes Python at runtime.

Timestamps produced by the injected clock are in 2031 and are compared exactly
by the Rust side; every other timestamp is real wall-clock time and is
normalized before comparison. Digest values are normalized too, because the
native Rust domain package digest is deliberately not the historical Python
source digest; the Rust test separately proves the digest *algorithm* by
recomputing the Python identities.
"""

from __future__ import annotations

import json
import re
import sqlite3
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any, Callable

REPO_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO_ROOT / "src"))

from kala_agent.application import (  # noqa: E402
    COMMITTED_REVISION_EVENT,
    KalaApplication,
)
from kala_agent.config import Settings  # noqa: E402

CLOCK_BASE = datetime(2031, 1, 1, 0, 0, 0, tzinfo=UTC)
CLOCK_STEP = timedelta(seconds=1)

HEX64 = re.compile(r"^[0-9a-f]{64}$")


def canonical(value: object) -> str:
    return json.dumps(
        value,
        allow_nan=False,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )


class Oracle:
    def __init__(self, data_dir: Path) -> None:
        self.data_dir = data_dir
        self.clock_calls = 0

        def clock() -> datetime:
            value = CLOCK_BASE + CLOCK_STEP * self.clock_calls
            self.clock_calls += 1
            return value

        self.application = KalaApplication(Settings(data_dir=data_dir), clock=clock)
        self.steps: dict[str, Any] = {}

    def record(self, name: str, operation: Callable[[], object]) -> None:
        try:
            value = operation()
        except Exception as error:  # noqa: BLE001 - the error shape is the observation
            self.steps[name] = {
                "ok": False,
                "class": type(error).__name__,
                "code": getattr(error, "code", None),
                "message": str(error),
            }
            return
        self.steps[name] = {"ok": True, "value": value}

    def jsonable(self, value: object) -> object:
        if value is None or isinstance(value, (bool, int, float, str)):
            return value
        if isinstance(value, (list, tuple, set, frozenset)):
            return [self.jsonable(item) for item in value]
        if isinstance(value, dict):
            return {str(key): self.jsonable(item) for key, item in value.items()}
        dump = getattr(value, "model_dump", None)
        if dump is not None:
            return self.jsonable(dump(mode="json"))
        return str(value)

    def run(self) -> dict[str, Any]:
        app = self.application
        steps = self.steps

        def expect(name: str, operation: Callable[[], object]) -> None:
            self.record(name, operation)

        expect("domain_packages", lambda: [self.jsonable(item) for item in app.list_domain_packages()])
        expect(
            "workflow_definitions",
            lambda: [self.jsonable(item) for item in app.list_workflow_definitions()],
        )
        expect("describe_mechanical", lambda: self.jsonable(app.describe_domain("mechanical")))
        expect(
            "catalog_building",
            lambda: [self.jsonable(item) for item in app.list_catalog_steps("building")],
        )
        expect(
            "eval_mechanical",
            lambda: self.jsonable(app.evaluate_domain_design("mechanical")),
        )
        expect(
            "eval_mechanical_args",
            lambda: self.jsonable(
                app.evaluate_domain_design(
                    "mechanical",
                    parameters={"load_kn": 9.0, "thickness_mm": 4.0},
                    design_id="eval-design",
                )
            ),
        )
        expect(
            "eval_building",
            lambda: self.jsonable(
                app.evaluate_domain_design(
                    "building",
                    parameters={"storey_height_m": 3.2},
                    design_id="building-design",
                )
            ),
        )
        expect(
            "eval_wing",
            lambda: self.jsonable(
                app.evaluate_domain_design(
                    "wing",
                    parameters={"span_m": 12.0},
                    design_id="wing-design",
                )
            ),
        )
        expect(
            "eval_bad_domain",
            lambda: self.jsonable(app.evaluate_domain_design("absent")),
        )
        expect(
            "eval_bad_params",
            lambda: self.jsonable(
                app.evaluate_domain_design(
                    "mechanical", parameters={"thickness_mm": 0.5}
                )
            ),
        )

        expect(
            "eval_bad_params_type",
            lambda: self.jsonable(
                app.evaluate_domain_design(
                    "mechanical", parameters={"length_mm": "abc"}
                )
            ),
        )
        expect(
            "eval_extra_params",
            lambda: self.jsonable(
                app.evaluate_domain_design("mechanical", parameters={"nope": 1})
            ),
        )

        r0 = None
        r1 = None
        r2_prepared = None
        r3_prepared = None

        def create_alpha() -> object:
            nonlocal r0
            r0 = app.create_design_revision(design_id="alpha", domain="mechanical")
            return self.jsonable(r0)

        def create_alpha_1() -> object:
            nonlocal r1
            r1 = app.create_design_revision(design_id="alpha", domain="mechanical")
            return self.jsonable(r1)

        def prepare_alpha_2() -> object:
            nonlocal r2_prepared
            r2_prepared = app.prepare_design_revision(design_id="alpha", domain="mechanical")
            return self.jsonable(r2_prepared)

        def prepare_alpha_3() -> object:
            nonlocal r3_prepared
            r3_prepared = app.prepare_design_revision(
                design_id="alpha",
                domain="mechanical",
                parameters={"load_kn": 8.0},
                summary="  a reviewed summary  ",
            )
            return self.jsonable(r3_prepared)

        expect("create_alpha_r0", create_alpha)
        expect("create_alpha_r1", create_alpha_1)
        expect("prepare_alpha_r2", prepare_alpha_2)
        expect(
            "get_prepared_r2",
            lambda: self.jsonable(app.get_revision_commit(r2_prepared.request_id)),
        )
        expect(
            "commit_r2",
            lambda: self.jsonable(
                app.commit_design_revision(
                    r2_prepared.request_id,
                    actor_id="op-a",
                    state_digest=r2_prepared.state_digest,
                    action_digest=r2_prepared.action_digest,
                )
            ),
        )
        expect("prepare_alpha_r3", prepare_alpha_3)
        expect(
            "reject_r3",
            lambda: self.jsonable(
                app.decide_revision_commit(
                    r3_prepared.request_id,
                    "reject",
                    actor_id="op-reject",
                    reason="  not reviewed  ",
                )
            ),
        )
        expect(
            "get_rejected_r3",
            lambda: self.jsonable(app.get_revision_commit(r3_prepared.request_id)),
        )
        expect(
            "list_alpha",
            lambda: [self.jsonable(item) for item in app.list_design_revisions("alpha")],
        )
        expect(
            "fork_refused",
            lambda: self.jsonable(
                app.create_design_revision(
                    design_id="alpha",
                    domain="mechanical",
                    parent_revision_id=r0.revision.revision_id,
                )
            ),
        )
        expect(
            "get_r2",
            lambda: self.jsonable(app.get_design_revision("rev-alpha-r2")),
        )
        expect(
            "stale_digest",
            lambda: self.jsonable(
                app.create_design_revision(
                    design_id="delta",
                    domain="mechanical",
                    state_digest="0" * 64,
                )
            ),
        )
        expect(
            "unknown_revision",
            lambda: self.jsonable(app.get_design_revision("rev-absent-r0")),
        )
        expect(
            "unknown_prepared",
            lambda: self.jsonable(app.get_revision_commit("commit-absent")),
        )
        expect(
            "unknown_domain_describe",
            lambda: self.jsonable(app.describe_domain("absent")),
        )
        extra = app.create_design_revision(
            design_id="extra",
            domain="mechanical",
            parent_provenance_ids=("prov-external",),
            transformation_ids=("zeta", "alpha", "zeta"),
        )
        expect("create_extra", lambda: self.jsonable(extra))
        expect(
            "get_extra",
            lambda: self.jsonable(app.get_design_revision("rev-extra-r0")),
        )
        expect(
            "list_alpha_limited",
            lambda: [self.jsonable(item) for item in app.list_design_revisions("alpha", limit=2)],
        )

        workflow_alpha = "workflow-alpha"
        expect(
            "workflow_run",
            lambda: self.jsonable(
                app.run_mechanical_reference_workflow(
                    workflow_id=workflow_alpha, design_id="alpha", actor_id="op-run"
                )
            ),
        )
        expect(
            "workflow_state",
            lambda: self.jsonable(app.get_workflow_state(workflow_alpha)),
        )
        expect(
            "workflow_checkpoints",
            lambda: [
                self.jsonable(item) for item in app.list_workflow_checkpoints(workflow_alpha)
            ],
        )
        expect(
            "approve_stale",
            lambda: self.jsonable(
                app.approve_workflow_checkpoint(workflow_alpha, state_digest="0" * 64)
            ),
        )

        checkpoint = app.get_workflow_state(workflow_alpha).pending_approval
        approved_state_digest = checkpoint.state_digest
        approved_action_digest = checkpoint.action_digest

        expect(
            "approve",
            lambda: self.jsonable(
                app.approve_workflow_checkpoint(
                    workflow_alpha,
                    actor_id="op-approve",
                    state_digest=approved_state_digest,
                    action_digest=approved_action_digest,
                )
            ),
        )
        expect(
            "workflow_state_done",
            lambda: self.jsonable(app.get_workflow_state(workflow_alpha)),
        )
        expect(
            "approve_again",
            lambda: self.jsonable(app.approve_workflow_checkpoint(workflow_alpha)),
        )
        expect(
            "workflow_checkpoints_after",
            lambda: [
                self.jsonable(item) for item in app.list_workflow_checkpoints(workflow_alpha)
            ],
        )
        expect(
            "run_again",
            lambda: self.jsonable(
                app.run_mechanical_reference_workflow(
                    workflow_id=workflow_alpha, design_id="alpha"
                )
            ),
        )
        expect(
            "revision_created_event",
            lambda: [
                self.jsonable(event.payload)
                for event in app.list_events(limit=1000, event_type=COMMITTED_REVISION_EVENT).events
                if event.aggregate_id == "rev-alpha-r3"
            ],
        )

        workflow_beta = "workflow-beta"
        expect(
            "rejected_run",
            lambda: self.jsonable(
                app.run_mechanical_reference_workflow(
                    workflow_id=workflow_beta, design_id="beta", actor_id="op-run"
                )
            ),
        )
        expect(
            "rejected_decision",
            lambda: self.jsonable(
                app.reject_workflow_checkpoint(
                    workflow_beta, actor_id="op-reject", reason="  declined  "
                )
            ),
        )
        expect(
            "rejected_state",
            lambda: self.jsonable(app.get_workflow_state(workflow_beta)),
        )
        expect(
            "rejected_runs",
            lambda: [self.jsonable(item) for item in app.list_design_revisions("beta")],
        )
        expect(
            "rejected_approve",
            lambda: self.jsonable(app.approve_workflow_checkpoint(workflow_beta)),
        )

        prepared_gamma = app.prepare_design_revision(design_id="gamma", domain="mechanical")
        expect("revoke", lambda: app.policy.revoke("design.write"))
        expect(
            "commit_revoked",
            lambda: self.jsonable(
                app.commit_design_revision(prepared_gamma.request_id, actor_id="op-gamma")
            ),
        )
        expect("grant", lambda: {"granted": app.policy.grant("design.write") is not None})
        expect(
            "commit_after_grant",
            lambda: self.jsonable(
                app.commit_design_revision(prepared_gamma.request_id, actor_id="op-gamma")
            ),
        )

        expect(
            "reserved_run",
            lambda: self.jsonable(
                app.run_mechanical_reference_workflow(
                    workflow_id="kernel-state:anything", design_id="alpha"
                )
            ),
        )
        expect(
            "reserved_state",
            lambda: self.jsonable(app.get_workflow_state("kernel-state:anything")),
        )
        expect(
            "bad_identifier",
            lambda: self.jsonable(
                app.create_design_revision(design_id="bad id!", domain="mechanical")
            ),
        )
        expect(
            "bad_limit",
            lambda: [self.jsonable(item) for item in app.list_design_revisions("alpha", limit=0)],
        )
        prepared_epsilon = app.prepare_design_revision(
            design_id="epsilon", domain="mechanical"
        )
        expect(
            "empty_reason",
            lambda: self.jsonable(
                app.decide_revision_commit(
                    prepared_epsilon.request_id, "approve", reason="   "
                )
            ),
        )
        expect(
            "get_pending_epsilon",
            lambda: self.jsonable(app.get_revision_commit(prepared_epsilon.request_id)),
        )

        page = app.list_events(limit=5)
        expect("events_page1", lambda: self.jsonable(page))
        expect(
            "events_page2",
            lambda: self.jsonable(app.list_events(after_sequence=page.next_after_sequence, limit=5)),
        )
        expect(
            "events_by_type",
            lambda: self.jsonable(app.list_events(limit=1000, event_type="revision.created")),
        )
        expect(
            "events_by_workflow",
            lambda: self.jsonable(
                app.list_events(limit=1000, workflow_id=workflow_alpha)
            ),
        )
        expect(
            "events_bad_type",
            lambda: self.jsonable(app.list_events(event_type="not a type")),
        )
        expect(
            "events_by_aggregate",
            lambda: self.jsonable(app.list_events(limit=1000, aggregate_id="rev-alpha-r3")),
        )
        expect(
            "bad_request_id",
            lambda: self.jsonable(app.get_revision_commit("commit bad!")),
        )
        expect(
            "bad_decision",
            lambda: self.jsonable(
                app.decide_revision_commit(r2_prepared.request_id, "maybe")
            ),
        )
        expect(
            "bad_actor",
            lambda: self.jsonable(
                app.approve_workflow_checkpoint(workflow_beta, actor_id="bad actor")
            ),
        )
        expect(
            "run_bad_design",
            lambda: self.jsonable(
                app.run_mechanical_reference_workflow(
                    workflow_id="workflow-zed", design_id="bad design"
                )
            ),
        )
        expect(
            "invalid_limit_high",
            lambda: [self.jsonable(item) for item in app.list_design_revisions("alpha", limit=1001)],
        )
        expect(
            "unknown_workflow_events",
            lambda: self.jsonable(app.list_events(limit=5, workflow_id="workflow-absent")),
        )

        verification = app.journal.verify_chain()
        expect(
            "verify_chain",
            lambda: {
                "event_count": verification.event_count,
                "head_sequence": verification.head_sequence,
                "head_event_digest": verification.head_event_digest,
            },
        )
        expect(
            "facts",
            lambda: {
                "offline": app.offline,
                "egress_configured": app.egress_configured,
                "provider_endpoint_declared": app.provider_endpoint_declared,
                "model_gateway_supplied": app.model_gateway_supplied,
                "provider_id": app.provider_id,
                "model_id": app.model_id,
                "domains": list(app.domains),
                "closed": app.closed,
            },
        )
        usage = app.budget.usage
        expect(
            "budget",
            lambda: {
                "model_calls": usage.model_calls,
                "tool_calls": usage.tool_calls,
                "total_tokens": usage.total_tokens,
                "artifact_bytes": usage.artifact_bytes,
                "duration_seconds": usage.duration_seconds,
                "cost": str(usage.cost),
                "replanning_attempts": usage.replanning_attempts,
            },
        )
        expect("grants", lambda: sorted(app.policy.grants))

        self.workflow_alpha = workflow_alpha
        self.r2_request_id = r2_prepared.request_id
        self.clock_calls_first = self.clock_calls
        trace: dict[str, Any] = {"steps": self.steps}
        trace["clock_calls_first"] = self.clock_calls

        # The restart: policy/budget state and the journal must survive.
        app.close()
        trace["closed"] = app.closed
        trace["closed_verify"] = self.record_call(
            "closed_verify", lambda: app.list_events().count
        )
        trace.update(self.collect_trace())

        reopened = KalaApplication(Settings(data_dir=self.data_dir), clock=lambda: CLOCK_BASE)
        restart: dict[str, Any] = {}
        restart["revisions_alpha"] = [
            self.jsonable(item) for item in reopened.list_design_revisions("alpha")
        ]
        restart["revisions_gamma"] = [
            self.jsonable(item) for item in reopened.list_design_revisions("gamma")
        ]
        restart["event_count"] = reopened.journal.verify_chain().event_count
        restart["grants"] = sorted(reopened.policy.grants)
        usage2 = reopened.budget.usage
        restart["budget"] = {
            "artifact_bytes": usage2.artifact_bytes,
            "model_calls": usage2.model_calls,
            "cost": str(usage2.cost),
        }
        restart["workflow_alpha"] = self.jsonable(reopened.get_workflow_state(workflow_alpha))
        restart["commit_r2"] = self.jsonable(reopened.get_revision_commit(self.r2_request_id))
        restart["checkpoint_records"] = [
            self.jsonable(item) for item in reopened.list_workflow_checkpoints(workflow_alpha)
        ]
        trace["restart"] = restart
        reopened.close()
        trace["clock_calls_total"] = self.clock_calls
        return trace

    def record_call(self, name: str, operation: Callable[[], object]) -> Any:
        try:
            value = operation()
        except Exception as error:  # noqa: BLE001
            return {
                "ok": False,
                "class": type(error).__name__,
                "code": getattr(error, "code", None),
                "message": str(error),
            }
        return {"ok": True, "value": value}

    def collect_trace(self) -> dict[str, Any]:
        database_path = Settings(data_dir=self.data_dir).database_path
        connection = sqlite3.connect(database_path)
        connection.row_factory = sqlite3.Row
        rows: dict[str, Any] = {}
        json_columns = {"model_json", "payload_json", "state_json"}
        for table in (
            "design_specs",
            "design_revisions",
            "approval_requests",
            "events",
            "workflow_snapshots",
            "tool_calls",
            "tool_results",
        ):
            entries = []
            for row in connection.execute(f"SELECT * FROM {table} ORDER BY rowid"):
                entry = {}
                for key in row.keys():
                    value = row[key]
                    if key in json_columns and isinstance(value, str):
                        entry[key] = json.loads(value)
                    else:
                        entry[key] = value
                entries.append(entry)
            rows[table] = entries
        rows["schema"] = [
            {"type": row["type"], "name": row["name"], "sql": row["sql"]}
            for row in connection.execute(
                "SELECT type, name, sql FROM sqlite_master ORDER BY type, name"
            )
        ]
        connection.close()

        artifacts = []
        root = Settings(data_dir=self.data_dir).artifact_dir
        for path in sorted(root.rglob("*")):
            if path.is_file():
                data = path.read_bytes()
                artifacts.append(
                    {
                        "parts": list(path.relative_to(root).parts),
                        "size": len(data),
                        "sha256": __import__("hashlib").sha256(data).hexdigest(),
                        "text": json.loads(data.decode("utf-8")),
                    }
                )
        artifacts.sort(
            key=lambda item: (
                str(item["text"].get("subject_id", "")) if isinstance(item["text"], dict) else "",
                str(item["text"].get("domain", "")) if isinstance(item["text"], dict) else "",
                item["size"],
                canonical(item["text"]),
            )
        )
        return {"rows": rows, "artifacts": artifacts}


def _lcg(seed: int):
    state = [seed]

    def next_value() -> int:
        state[0] = (1103515245 * state[0] + 12345) % 2147483648
        return state[0]

    return next_value


def _mechanical_parameters(next_value) -> dict[str, Any]:
    materials = ["aluminium-6061-t6", "steel-s355", "titanium-ti6al4v"]
    length = 100 + next_value() % 400
    width = 60 + next_value() % 240
    hole_diameter = 2.0 + (next_value() % 80) / 10.0
    hole_count = 1 + next_value() % 6
    while hole_count * hole_diameter > length:
        hole_count -= 1
    return {
        "length_mm": length,
        "width_mm": width,
        "thickness_mm": 1.0 + (next_value() % 180) / 10.0,
        "hole_diameter_mm": hole_diameter,
        "hole_count": hole_count,
        "material": materials[next_value() % 3],
        "load_kn": (next_value() % 2000) / 10.0,
        "load_points": 1 + next_value() % 8,
        "minimum_edge_margin_mm": (next_value() % 40) / 10.0,
        "minimum_thickness_mm": 0.5 + (next_value() % 100) / 10.0,
        "maximum_stress_mpa": 50 + next_value() % 400,
        "target_safety_factor": 1.0 + (next_value() % 40) / 10.0,
    }


def _building_parameters(next_value) -> dict[str, Any]:
    return {
        "storey_count": 1 + next_value() % 40,
        "storey_height_m": 2.0 + (next_value() % 40) / 10.0,
        "gross_floor_area_m2": 50 + next_value() % 1950,
        "perimeter_m": 10 + next_value() % 190,
        "window_to_wall_ratio": (next_value() % 90) / 100.0,
        "heating_degree_days_k": 100 + next_value() % 39000,
    }


def _wing_parameters(next_value) -> dict[str, Any]:
    root = 0.5 + (next_value() % 40) / 10.0
    return {
        "span_m": 2.0 + (next_value() % 58),
        "root_chord_m": root,
        "tip_chord_m": root * (0.1 + (next_value() % 8) / 10.0),
        "sweep_deg": -5.0 + (next_value() % 65),
        "dihedral_deg": -10.0 + (next_value() % 50),
        "spanwise_stations": 2 + next_value() % 30,
    }


CORPUS_INVALID = (
    {"length_mm": "not-a-number"},
    {"hole_count": 2.5},
    {"length_mm": 10},
    {"width_mm": 1000},
    {"material": "unobtainium"},
    {"nope": 1},
    {"length_mm": None},
    {"hole_count": True},
    {"load_points": 0},
    {"thickness_mm": "nan"},
    {"length_mm": [1, 2]},
    {"storey_count": 0},
)


def corpus(data_dir: Path) -> dict[str, Any]:
    oracle = Oracle(data_dir)
    app = oracle.application

    def call(operation):
        return oracle.record_call("corpus", operation)

    next_value = _lcg(20261019)
    mechanical = [_mechanical_parameters(next_value) for _ in range(16)]
    building = [_building_parameters(next_value) for _ in range(8)]
    wing = [_wing_parameters(next_value) for _ in range(8)]
    cases = []
    for index, parameters in enumerate(mechanical):
        cases.append(
            {
                "kind": "mechanical-evaluate",
                "index": index,
                "parameters": parameters,
                "result": call(
                    lambda parameters=parameters, index=index: oracle.jsonable(
                        app.evaluate_domain_design(
                            "mechanical",
                            parameters=parameters,
                            design_id=f"corpus-{index}",
                        )
                    )
                ),
            }
        )
    for index, parameters in enumerate(building):
        cases.append(
            {
                "kind": "building-evaluate",
                "index": index,
                "parameters": parameters,
                "result": call(
                    lambda parameters=parameters, index=index: oracle.jsonable(
                        app.evaluate_domain_design(
                            "building",
                            parameters=parameters,
                            design_id=f"corpus-building-{index}",
                        )
                    )
                ),
            }
        )
    for index, parameters in enumerate(wing):
        cases.append(
            {
                "kind": "wing-evaluate",
                "index": index,
                "parameters": parameters,
                "result": call(
                    lambda parameters=parameters, index=index: oracle.jsonable(
                        app.evaluate_domain_design(
                            "wing",
                            parameters=parameters,
                            design_id=f"corpus-wing-{index}",
                        )
                    )
                ),
            }
        )
    for index, parameters in enumerate(CORPUS_INVALID):
        cases.append(
            {
                "kind": "invalid-evaluate",
                "index": index,
                "parameters": parameters,
                "result": call(
                    lambda parameters=parameters: oracle.jsonable(
                        app.evaluate_domain_design("mechanical", parameters=parameters)
                    )
                ),
            }
        )
    for index in (0, 5, 10, 15):
        parameters = mechanical[index]
        cases.append(
            {
                "kind": "commit",
                "index": index,
                "parameters": parameters,
                "result": call(
                    lambda parameters=parameters, index=index: oracle.jsonable(
                        app.create_design_revision(
                            design_id=f"corpus-commit-{index}",
                            domain="mechanical",
                            parameters=parameters,
                            transformation_ids=("corpus-a", "corpus-b"),
                        )
                    )
                ),
            }
        )
        cases.append(
            {
                "kind": "read-commit",
                "index": index,
                "result": call(
                    lambda index=index: oracle.jsonable(
                        app.get_design_revision(f"rev-corpus-commit-{index}-r0")
                    )
                ),
            }
        )
    app.close()
    trace = oracle.collect_trace()
    trace["cases"] = cases
    return trace


def wire(data_dir: Path) -> dict[str, Any]:
    """Return the exact Pydantic wire JSON of the read models."""
    oracle = Oracle(data_dir)
    app = oracle.application
    detail = app.describe_domain("mechanical")
    report = app.evaluate_domain_design(
        "mechanical", parameters={"load_kn": 2.5}, design_id="wire-design"
    )
    prepared = app.prepare_design_revision(
        design_id="wire-design", domain="mechanical"
    )
    committed = app.commit_design_revision(prepared.request_id)
    commit_record = app.get_revision_commit(prepared.request_id)
    run = app.run_mechanical_reference_workflow(
        workflow_id="workflow-wire", design_id="wire-design"
    )
    state = app.get_workflow_state("workflow-wire")
    checkpoints = app.list_workflow_checkpoints("workflow-wire")
    packages = app.list_domain_packages()
    definitions = app.list_workflow_definitions()
    steps = app.list_catalog_steps("mechanical")
    def transport(model) -> str:
        # The FastAPI/Starlette form: json.dumps of model_dump(mode="json").
        return json.dumps(
            model.model_dump(mode="json"),
            allow_nan=False,
            ensure_ascii=False,
            separators=(",", ":"),
        )

    strings = {
        "detail": transport(detail),
        "report": transport(report),
        "prepared": transport(prepared),
        "committed": transport(committed),
        "commit_record": transport(commit_record),
        "run": transport(run),
        "state": transport(state),
        "checkpoint": transport(checkpoints[0]),
        "package": transport(packages[1]),
        "definition": transport(definitions[1]),
        "step": transport(steps[0]),
    }
    app.close()
    return {"strings": strings}


def seed(data_dir: Path) -> dict[str, Any]:
    """Write a data directory from the Python side for the cross-reader."""
    oracle = Oracle(data_dir)
    app = oracle.application
    revisions = [
        app.create_design_revision(design_id="cross", domain="mechanical"),
        app.create_design_revision(design_id="cross", domain="mechanical"),
    ]
    run = app.run_mechanical_reference_workflow(
        workflow_id="workflow-cross", design_id="cross"
    )
    decided = app.approve_workflow_checkpoint("workflow-cross", actor_id="py-operator")
    app.policy.revoke("design.write")
    pending = app.prepare_design_revision(
        design_id="cross-pending", domain="mechanical"
    )
    steps = {
        "revision_ids": [item.revision.revision_id for item in revisions],
        "run_checkpoint": oracle.jsonable(run.checkpoint),
        "decision_status": decided.status.value,
        "decision_revision": oracle.jsonable(decided.revision),
        "events": app.journal.verify_chain().event_count,
        "grants": sorted(app.policy.grants),
        "budget": str(app.budget.usage.cost),
        "artifact_bytes": app.budget.usage.artifact_bytes,
        "pending_request": pending.request_id,
        "pending_status": pending.status.value,
    }
    app.close()
    trace = oracle.collect_trace()
    trace["steps"] = steps
    return trace


def inspect(data_dir: Path) -> dict[str, Any]:
    """Read a data directory (possibly written by Rust) and extend it."""
    oracle = Oracle(data_dir)
    app = oracle.application
    chain_before = app.journal.verify_chain().event_count
    grants_before = sorted(app.policy.grants)
    prepared_events = app.list_events(limit=1000, event_type="revision.prepared").events
    pending_status = None
    pending_request = None
    for event in prepared_events:
        payload = event.payload or {}
        request_id = payload.get("request_id")
        if isinstance(request_id, str):
            record = app.get_revision_commit(request_id)
            if record.status.value == "pending":
                pending_request = request_id
                pending_status = record.status.value
                break
    app.policy.grant("design.write")
    revisions = app.list_design_revisions("cross")
    last = revisions[-1]
    artifact = last.revision.artifacts[0]
    artifact_bytes = app.artifact_store.get_bytes(artifact.digest)
    workflow_state = app.get_workflow_state("workflow-cross")
    created = app.create_design_revision(
        design_id="cross", domain="mechanical", summary="cross read write"
    )
    run = app.run_mechanical_reference_workflow(
        workflow_id="workflow-cross-2", design_id="cross"
    )
    approved = app.approve_workflow_checkpoint("workflow-cross-2", actor_id="py-cross")
    steps = {
        "chain_before": chain_before,
        "grants_before": grants_before,
        "pending_request": pending_request,
        "pending_status": pending_status,
        "artifact": {
            "digest": artifact.digest,
            "sha256": __import__("hashlib").sha256(artifact_bytes).hexdigest(),
            "size": len(artifact_bytes),
        },
        "revision_ids": [item.revision.revision_id for item in revisions],
        "last_revision": oracle.jsonable(last),
        "workflow_state": oracle.jsonable(workflow_state),
        "created": oracle.jsonable(created),
        "run_checkpoint": oracle.jsonable(run.checkpoint),
        "approved_status": approved.status.value,
        "approved_revision": oracle.jsonable(approved.revision),
        "grants": sorted(app.policy.grants),
        "final_ids": [
            item.revision.revision_id for item in app.list_design_revisions("cross")
        ],
        "events": app.journal.verify_chain().event_count,
    }
    app.close()
    trace = oracle.collect_trace()
    trace["steps"] = steps
    return trace


def main() -> int:
    scenario = sys.argv[1]
    data_dir = Path(sys.argv[2])
    data_dir.mkdir(parents=True, exist_ok=True)
    if scenario == "main":
        oracle = Oracle(data_dir)
        trace = oracle.run()
    elif scenario == "seed":
        trace = seed(data_dir)
    elif scenario == "inspect":
        trace = inspect(data_dir)
    elif scenario == "corpus":
        trace = corpus(data_dir)
    elif scenario == "wire":
        trace = wire(data_dir)
    else:
        raise SystemExit(f"unknown scenario: {scenario}")
    sys.stdout.write(canonical(trace))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
