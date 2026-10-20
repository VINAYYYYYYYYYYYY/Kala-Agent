"""Tests for the Studio session journal and its artifact store.

ADR-0009 acceptance criterion 5 is exercised here at library level: the
chain verifies after a design turn, and mutating a stored payload byte makes
verification fail and names the first broken sequence. The tests also pin
the two conventions the journal must not reinvent: the canonical JSON form
of a payload and the chain preimage, which is recomputed here with the core
:func:`~kala_agent.persistence.chain.chain_digest` function.
"""

from __future__ import annotations

import ast
import hashlib
import json
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from kala_agent.persistence.chain import (
    GENESIS_PREVIOUS_DIGEST,
    EventChainFields,
    JournalIntegrityError,
    chain_digest,
)
from kala_agent.studio import session_journal as session_journal_module
from kala_agent.studio.session_journal import (
    DEFAULT_AGGREGATE_ID,
    JournalEventType,
    JournalVerification,
    StudioSessionJournal,
)


def _imported_modules(source: str) -> set[str]:
    """Return the absolute module names the given source imports."""
    names: set[str] = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names.add(node.module)
    return names


# --- append ---


def test_append_returns_the_sequence_and_the_payload_digest(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    event = journal.append(JournalEventType.CHAT_TURN, {"text": "hello"})

    assert event.sequence == 1
    assert event.event_type == "chat.turn"
    assert event.digest == hashlib.sha256(event.payload_json.encode("utf-8")).hexdigest()
    assert event.previous_event_digest == GENESIS_PREVIOUS_DIGEST
    assert event.payload == {"text": "hello"}


def test_append_uses_the_core_chain_preimage(tmp_path: Path) -> None:
    """The Studio digest is the core chain digest, not a second scheme."""
    journal = StudioSessionJournal(tmp_path)
    event = journal.append("design.intent", {"template": "l_bracket", "width_mm": 60})

    expected = chain_digest(
        EventChainFields(
            sequence=1,
            event_type="design.intent",
            aggregate_id=DEFAULT_AGGREGATE_ID,
            workflow_id=None,
            run_id=None,
            occurred_at=event.occurred_at,
            payload_digest=event.digest,
            previous_event_digest=GENESIS_PREVIOUS_DIGEST,
        )
    )
    assert event.event_digest == expected


def test_append_stores_canonical_payload_json(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    event = journal.append("chat.turn", {"b": 1, "a": [2, {"d": 4, "c": 3}]})

    assert event.payload_json == '{"a":[2,{"c":3,"d":4}],"b":1}'


def test_append_refuses_a_payload_that_is_not_json(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    with pytest.raises(ValueError):
        journal.append("chat.turn", {"value": object()})

    assert journal.verify().event_count == 0


def test_append_refuses_an_unsafe_event_type(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    with pytest.raises(ValueError):
        journal.append("not a safe event type", {"text": "hello"})


# --- page ---


def test_page_returns_events_in_sequence_order_after_the_cursor(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    for index in range(5):
        journal.append("chat.turn", {"index": index})

    first = journal.page(0, 3)
    second = journal.page(3, 10)
    third = journal.page(5, 10)

    assert [event.sequence for event in first] == [1, 2, 3]
    assert [event.sequence for event in second] == [4, 5]
    assert third == ()
    assert [event.payload_json for event in first] == [
        '{"index":0}',
        '{"index":1}',
        '{"index":2}',
    ]


def test_page_is_bounded_by_the_maximum_page_size(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path, max_page_size=4)
    for index in range(10):
        journal.append("chat.turn", {"index": index})

    page = journal.page(0, 1_000)

    assert len(page) == 4
    assert [event.sequence for event in page] == [1, 2, 3, 4]


def test_page_rejects_an_invalid_cursor_or_limit(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    with pytest.raises(ValueError):
        journal.page(-1, 10)
    with pytest.raises(ValueError):
        journal.page(0, 0)
    with pytest.raises(TypeError):
        journal.page(0, "10")  # type: ignore[arg-type]


# --- verify ---


def test_verify_reports_an_intact_chain(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    events = [journal.append("chat.turn", {"index": index}) for index in range(3)]

    verification = journal.verify()

    assert isinstance(verification, JournalVerification)
    assert verification.ok is True
    assert verification.event_count == 3
    assert verification.head_digest == events[-1].event_digest
    assert verification.first_bad_sequence is None
    assert dict(verification) == {
        "ok": True,
        "event_count": 3,
        "head_digest": events[-1].event_digest,
        "first_bad_sequence": None,
    }


def test_an_empty_journal_verifies(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    verification = journal.verify()

    assert verification.ok is True
    assert verification.event_count == 0
    assert verification.head_digest is None
    assert verification.first_bad_sequence is None


def test_verify_detects_a_tampered_payload(tmp_path: Path) -> None:
    """One mutated payload byte breaks the chain at exactly that event."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.append("design.measured", {"volume_mm3": 35571.416047})
    journal.append("design.checked", {"verified": True})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"volume_mm3":35571.416048}', 2),
        )
        connection.commit()
    finally:
        connection.close()

    verification = journal.verify()

    assert verification.ok is False
    assert verification.first_bad_sequence == 2
    assert verification.event_count == 3
    assert verification.head_digest == journal.get(1).event_digest  # type: ignore[union-attr]


def test_verify_detects_a_tampered_event_digest(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.append("design.measured", {"volume_mm3": 1.0})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET event_digest = ? WHERE sequence = ?",
            ("f" * 64, 2),
        )
        connection.commit()
    finally:
        connection.close()

    verification = journal.verify()

    assert verification.ok is False
    assert verification.first_bad_sequence == 2


def test_verify_detects_a_dropped_event(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.append("design.measured", {"volume_mm3": 1.0})
    journal.append("design.checked", {"verified": True})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute("DELETE FROM studio_events WHERE sequence = 2")
        connection.commit()
    finally:
        connection.close()

    verification = journal.verify()

    assert verification.ok is False
    assert verification.first_bad_sequence == 2


def test_a_tampered_journal_is_not_extended(tmp_path: Path) -> None:
    """A write may not grow a chain that has already been broken."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"text":"hellp"}', 1),
        )
        connection.commit()
    finally:
        connection.close()

    with pytest.raises(JournalIntegrityError):
        journal.append("chat.turn", {"text": "again"})

    assert journal.verify().event_count == 1


def test_a_tampered_journal_is_still_readable(tmp_path: Path) -> None:
    """The history stays readable so the UI can show what happened."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ("not json at all", 1),
        )
        connection.commit()
    finally:
        connection.close()

    events = journal.page(0, 10)

    assert len(events) == 1
    assert events[0].payload is None
    assert events[0].payload_json == "not json at all"


# --- artifacts ---


def test_artifact_round_trip_returns_the_sha256(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    data = b"ISO-10303-21;\nEND-ISO-10303-21;\n"

    digest = journal.put_artifact(data)

    assert digest == hashlib.sha256(data).hexdigest()
    assert len(digest) == 64
    assert journal.get_artifact(digest) == data
    assert journal.get_artifact_text(digest) == data.decode("utf-8")
    assert journal.has_artifact(digest) is True


def test_the_same_bytes_are_stored_once(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    first = journal.put_artifact(b"same bytes")
    second = journal.put_artifact(b"same bytes")

    assert first == second
    assert journal.get_artifact(first) == b"same bytes"


def test_an_unknown_artifact_is_refused(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)

    with pytest.raises(FileNotFoundError):
        journal.get_artifact("0" * 64)


def test_the_artifact_size_limit_is_enforced(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path, max_artifact_bytes=64)

    with pytest.raises(ValueError):
        journal.put_artifact(b"x" * 65)

    assert journal.has_artifact(hashlib.sha256(b"x" * 64).hexdigest()) is False


# --- limits, placement, and concurrency ---


def test_the_payload_size_limit_is_enforced_and_writes_nothing(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path, max_payload_bytes=128)

    with pytest.raises(ValueError):
        journal.append("chat.turn", {"text": "x" * 512})

    verification = journal.verify()
    assert verification.ok is True
    assert verification.event_count == 0


def test_the_journal_lives_in_the_supplied_data_directory(tmp_path: Path) -> None:
    data_dir = tmp_path / "studio-data"
    journal = StudioSessionJournal(data_dir)
    journal.append("chat.turn", {"text": "hello"})

    assert journal.database_path == data_dir.resolve() / "studio-journal.sqlite3"
    assert journal.database_path.is_file()
    assert journal.artifact_root == (data_dir.resolve() / "artifacts")
    assert journal.data_dir == data_dir.resolve()


def test_the_journal_never_reaches_the_core_application(tmp_path: Path) -> None:
    """The Studio journal is standalone: it composes no core application."""
    source = Path(session_journal_module.__file__).read_text(encoding="utf-8")
    imported = _imported_modules(source)

    assert "kala_agent.application" not in imported
    assert not any(name.startswith("kala_agent.application") for name in imported)
    assert not any(name.startswith("kala_agent.runtime") for name in imported)
    assert not any(name.startswith("kala_agent.tools") for name in imported)


def test_concurrent_appends_keep_the_chain_consistent(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    workers = 8
    per_worker = 10
    failures: list[BaseException] = []

    def worker(index: int) -> None:
        try:
            for step in range(per_worker):
                journal.append("chat.turn", {"worker": index, "step": step})
        except BaseException as error:  # pragma: no cover - reported through `failures`
            failures.append(error)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        list(pool.map(worker, range(workers)))

    assert failures == []
    verification = journal.verify()
    assert verification.ok is True
    assert verification.event_count == workers * per_worker
    events = journal.page(0, workers * per_worker)
    assert [event.sequence for event in events] == list(range(1, workers * per_worker + 1))


def test_readers_and_writers_share_the_journal_safely(tmp_path: Path) -> None:
    """The FastAPI thread pool may read while another thread appends."""
    journal = StudioSessionJournal(tmp_path)
    failures: list[BaseException] = []

    def writer(index: int) -> None:
        try:
            for step in range(10):
                journal.append("chat.turn", {"writer": index, "step": step})
        except BaseException as error:  # pragma: no cover - reported through `failures`
            failures.append(error)

    def reader() -> None:
        try:
            for _ in range(10):
                journal.page(0, 5)
                journal.verify()
        except BaseException as error:  # pragma: no cover - reported through `failures`
            failures.append(error)

    with ThreadPoolExecutor(max_workers=6) as pool:
        futures = [pool.submit(writer, index) for index in range(4)]
        futures.extend(pool.submit(reader) for _ in range(2))
        for future in futures:
            future.result()

    assert failures == []
    verification = journal.verify()
    assert verification.ok is True
    assert verification.event_count == 40


def test_the_stored_payload_is_the_canonical_bytes_that_were_hashed(tmp_path: Path) -> None:
    journal = StudioSessionJournal(tmp_path)
    payload = {"z": 1, "a": {"n": None, "m": True}}

    event = journal.append("chat.turn", payload)

    assert event.payload_json == json.dumps(payload, sort_keys=True, separators=(",", ":"))
    assert event.digest == hashlib.sha256(event.payload_json.encode("utf-8")).hexdigest()


# --- the cached head: what an append verifies, and what it costs ---


def test_appends_after_a_long_history_do_not_walk_the_whole_chain(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """An append verifies the newest row, not the whole chain.

    The chain is walked once, when the journal is opened, and the verified
    head is cached from then on. A steady stream of appends must not walk
    it again: a journal that persists on disk across sessions would
    otherwise cost more and more per event.
    """
    journal = StudioSessionJournal(tmp_path)
    for index in range(2000):
        journal.append("chat.turn", {"index": index})

    walks: list[int] = []
    original = journal._walk_chain

    def spy(connection: sqlite3.Connection) -> object:
        walks.append(1)
        return original(connection)

    monkeypatch.setattr(journal, "_walk_chain", spy)
    for index in range(100):
        journal.append("chat.turn", {"index": index})

    assert walks == []
    assert journal.verify().event_count == 2100
    assert journal.verify().ok is True


def test_a_tampered_head_is_detected_at_the_next_append(tmp_path: Path) -> None:
    """Editing the newest event is caught by the next write."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.append("design.checked", {"verified": True})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"verified":false}', 2),
        )
        connection.commit()
    finally:
        connection.close()

    with pytest.raises(JournalIntegrityError):
        journal.append("chat.turn", {"text": "again"})

    verification = journal.verify()
    assert verification.ok is False
    assert verification.first_bad_sequence == 2
    assert verification.event_count == 2


def test_a_tampered_middle_is_detected_by_verify_and_on_reopen(tmp_path: Path) -> None:
    """An edit in the middle needs a full walk: verify(), then the next open.

    The newest row still verifies against itself, so only a walk of the
    whole chain can find the break. The journal therefore reports it at the
    next verification and refuses to extend the chain after a reopen.
    """
    journal = StudioSessionJournal(tmp_path)
    for index in range(5):
        journal.append("chat.turn", {"index": index})

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"index":999}', 3),
        )
        connection.commit()
    finally:
        connection.close()

    verification = journal.verify()
    assert verification.ok is False
    assert verification.first_bad_sequence == 3
    assert verification.event_count == 5
    journal.close()

    reopened = StudioSessionJournal(tmp_path)
    try:
        reopened_verification = reopened.verify()
        assert reopened_verification.ok is False
        assert reopened_verification.first_bad_sequence == 3
        with pytest.raises(JournalIntegrityError):
            reopened.append("chat.turn", {"index": 5})
    finally:
        reopened.close()


def test_a_second_instance_on_the_same_file_extends_the_chain(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Two journals on one file extend one chain; they never fork it."""
    first = StudioSessionJournal(tmp_path)
    first.append("chat.turn", {"text": "one"})

    second = StudioSessionJournal(tmp_path)
    second.append("chat.turn", {"text": "two"})

    walks: list[int] = []
    original = first._walk_chain

    def spy(connection: sqlite3.Connection) -> object:
        walks.append(1)
        return original(connection)

    monkeypatch.setattr(first, "_walk_chain", spy)
    event = first.append("chat.turn", {"text": "three"})

    # The cached head no longer matched the file, so the chain was re-walked.
    assert walks
    assert event.sequence == 3

    events = first.page(0, 10)
    assert [item.payload_json for item in events] == [
        '{"text":"one"}',
        '{"text":"two"}',
        '{"text":"three"}',
    ]
    assert events[2].previous_event_digest == events[1].event_digest

    verification = first.verify()
    assert verification.ok is True
    assert verification.event_count == 3

    # The refreshed head lets the first instance carry on from the new head.
    assert second.append("chat.turn", {"text": "four"}).sequence == 4
    assert first.verify().event_count == 4
    first.close()
    second.close()


def test_a_broken_chain_is_not_extended_after_a_reopen(tmp_path: Path) -> None:
    """A journal opened on a broken chain refuses every write."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.append("chat.turn", {"text": "again"})
    journal.close()

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"text":"hellp"}', 1),
        )
        connection.commit()
    finally:
        connection.close()

    reopened = StudioSessionJournal(tmp_path)
    try:
        with pytest.raises(JournalIntegrityError):
            reopened.append("chat.turn", {"text": "third"})
        assert reopened.verify().first_bad_sequence == 1
        assert reopened.page(0, 10)[0].sequence == 1
    finally:
        reopened.close()


def test_a_repaired_chain_is_writable_again_after_a_verification(tmp_path: Path) -> None:
    """The refusal is lifted when a later verification finds the chain intact."""
    journal = StudioSessionJournal(tmp_path)
    journal.append("chat.turn", {"text": "hello"})
    journal.close()

    connection = sqlite3.connect(journal.database_path)
    try:
        connection.execute(
            "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
            ('{"text":"hellp"}', 1),
        )
        connection.commit()
    finally:
        connection.close()

    reopened = StudioSessionJournal(tmp_path)
    try:
        with pytest.raises(JournalIntegrityError):
            reopened.append("chat.turn", {"text": "again"})

        connection = sqlite3.connect(reopened.database_path)
        try:
            connection.execute(
                "UPDATE studio_events SET payload_json = ? WHERE sequence = ?",
                ('{"text":"hello"}', 1),
            )
            connection.commit()
        finally:
            connection.close()

        assert reopened.verify().ok is True
        event = reopened.append("chat.turn", {"text": "again"})
        assert event.sequence == 2
    finally:
        reopened.close()
