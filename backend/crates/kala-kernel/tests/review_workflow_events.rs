//! Independent review probe 4 for workflow: event-sink panics/failures and
//! re-entrancy, plus concurrent snapshots that must never be torn.

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use kala_contracts::{ContractModel, Text, WorkflowDefinition, WorkflowStatus};
use kala_json::CanonicalValue;
use kala_kernel::workflow::{
    EventCallbackSink, WorkflowEngine, WorkflowEvent, WorkflowEventSink, WorkflowState,
};
use support::{array, object, simple_definition, text};

fn definition(value: &CanonicalValue) -> WorkflowDefinition {
    WorkflowDefinition::from_value(value).expect("definition")
}

fn long_definition(steps: usize) -> WorkflowDefinition {
    let states: Vec<CanonicalValue> = (0..=steps)
        .map(|index| text(&format!("s{index}")))
        .collect();
    let transitions: Vec<CanonicalValue> = (0..steps)
        .map(|index| {
            object(vec![
                ("checkpoint_type", CanonicalValue::Null),
                ("from_state", text(&format!("s{index}"))),
                ("step_id", text(&format!("step-{index}"))),
                ("to_state", text(&format!("s{}", index + 1))),
                ("transition_id", text(&format!("transition-{index}"))),
            ])
        })
        .collect();
    definition(&object(vec![
        (
            "available_step_ids",
            array(
                (0..steps)
                    .map(|index| text(&format!("step-{index}")))
                    .collect(),
            ),
        ),
        ("definition_id", text("long-workflow")),
        ("initial_state", text("s0")),
        ("max_replanning_attempts", support::int(1)),
        ("name", text("Long workflow")),
        ("states", array(states)),
        ("transitions", array(transitions)),
        ("version", text("1.0.0")),
    ]))
}

struct PanicOn {
    event_type: &'static str,
    armed: AtomicBool,
}

impl PanicOn {
    fn new(event_type: &'static str) -> Self {
        Self {
            event_type,
            armed: AtomicBool::new(true),
        }
    }
}

impl WorkflowEventSink for PanicOn {
    fn emit(&self, event: &WorkflowEvent) -> Result<(), kala_kernel::WorkflowError> {
        if event.event_type.to_string_lossy() == self.event_type
            && self.armed.swap(false, Ordering::SeqCst)
        {
            panic!("review sink panic on {}", self.event_type);
        }
        Ok(())
    }
}

#[test]
fn panicking_sink_preserves_source_mutation_order_and_leaves_engine_usable() {
    // A panic in the `workflow.created` emit happens after the source stores
    // the state, so the state remains Created even though start panicked.
    let engine = WorkflowEngine::new(definition(&simple_definition(false)))
        .with_sink(Arc::new(PanicOn::new("workflow.created")));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        engine.start(&Text::new("workflow-1"), None)
    }));
    assert!(outcome.is_err(), "the sink panic must propagate");
    let created = engine.state().expect("start stored before emitting");
    assert_eq!(created.status(), WorkflowStatus::Created);
    // The engine is not poisoned and an idempotent start returns the state.
    let restarted = engine
        .start(&Text::new("workflow-1"), None)
        .expect("start after panic");
    assert_eq!(restarted, created);

    // A panic in `workflow.transition` happens before `store`, so the state
    // remains Created and no history is recorded.
    let engine = WorkflowEngine::new(definition(&simple_definition(false)))
        .with_sink(Arc::new(PanicOn::new("workflow.transition")));
    engine.start(&Text::new("workflow-1"), None).expect("start");
    let outcome = catch_unwind(AssertUnwindSafe(|| engine.run(None, None, None, None)));
    assert!(outcome.is_err(), "the sink panic must propagate");
    let after = engine.state().expect("state");
    assert_eq!(after.status(), WorkflowStatus::Created);
    assert!(after.history().is_empty());
    assert!(after.completed_step_ids().is_empty());
    // The panic is consumed; the same engine can finish the workflow.
    let completed = engine.run(None, None, None, None).expect("run after panic");
    assert_eq!(completed.status(), WorkflowStatus::Completed);
    assert_eq!(completed.history().len(), 1);

    // A panic in `workflow.approval_resolved` happens before the approval is
    // stored, so the pending checkpoint is still Pending.
    let engine = WorkflowEngine::new(definition(&simple_definition(true)))
        .with_sink(Arc::new(PanicOn::new("workflow.approval_resolved")));
    engine.start(&Text::new("workflow-1"), None).expect("start");
    engine.run(None, None, None, None).expect("checkpoint");
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        engine.approve_checkpoint(None, None, &Default::default())
    }));
    assert!(outcome.is_err(), "the sink panic must propagate");
    let waiting = engine.state().expect("state");
    assert_eq!(
        waiting.pending_approval().expect("pending").status(),
        kala_kernel::ApprovalStatus::Pending
    );
    assert_eq!(waiting.status(), WorkflowStatus::WaitingForApproval);
}

#[test]
fn reentrant_sink_can_read_and_mutate_without_deadlock() {
    type Engine = WorkflowEngine;
    let holder: Arc<Mutex<Weak<Engine>>> = Arc::new(Mutex::new(Weak::new()));
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let cancel_once = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));

    let sink = Arc::new(EventCallbackSink::new({
        let holder = holder.clone();
        let seen = seen.clone();
        let cancel_once = cancel_once.clone();
        let reads = reads.clone();
        move |event: &WorkflowEvent| {
            seen.lock()
                .expect("seen")
                .push(event.event_type.to_string_lossy());
            let upgraded = { holder.lock().expect("holder").upgrade() };
            if let Some(engine) = upgraded {
                // All three acquire the engine's state mutex. A non-reentrant
                // lock held across emit would deadlock here.
                engine.state().expect("read state");
                engine.serialize(None).expect("read serialize");
                engine.state_digest(None).expect("read digest");
                reads.fetch_add(1, Ordering::SeqCst);
                if event.event_type.to_string_lossy() == "workflow.created"
                    && !cancel_once.swap(true, Ordering::SeqCst)
                {
                    engine
                        .cancel(None, Some(&Text::new("reentrant")))
                        .expect("reentrant cancel");
                }
            }
            Ok(())
        }
    }));
    let engine =
        Arc::new(WorkflowEngine::new(definition(&simple_definition(false))).with_sink(sink));
    *holder.lock().expect("holder") = Arc::downgrade(&engine);

    let (sender, receiver) = mpsc::channel();
    let worker = engine.clone();
    let handle = thread::spawn(move || {
        let result = worker.start(&Text::new("workflow-1"), None);
        let _ = sender.send(result.map(|state| state.status()));
    });
    let started = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("reentrant sink deadlocked");
    handle.join().expect("thread");
    assert_eq!(started.expect("start"), WorkflowStatus::Created);
    // The nested cancel stored a cancelled state after the created emit.
    assert_eq!(
        engine.state().expect("state").status(),
        WorkflowStatus::Cancelled
    );
    assert_eq!(
        *seen.lock().expect("seen"),
        vec![
            "workflow.created".to_owned(),
            "workflow.cancelled".to_owned()
        ]
    );
    assert!(reads.load(Ordering::SeqCst) >= 2);
}

#[test]
fn concurrent_snapshots_restore_and_are_never_torn() {
    let definition = long_definition(24);
    let engine = Arc::new(WorkflowEngine::new(definition.clone()));
    engine.start(&Text::new("workflow-1"), None).expect("start");
    let done = Arc::new(AtomicBool::new(false));
    let snapshots = Arc::new(AtomicUsize::new(0));

    let writer_engine = engine.clone();
    let writer_done = done.clone();
    let writer = thread::spawn(move || {
        for _ in 0..24 {
            writer_engine
                .run(None, None, None, None)
                .expect("transition");
        }
        writer_done.store(true, Ordering::Release);
    });

    let mut readers = Vec::new();
    for _ in 0..6 {
        let engine = engine.clone();
        let done = done.clone();
        let snapshots = snapshots.clone();
        let definition = definition.clone();
        readers.push(thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if let Some(snapshot) = engine.state() {
                    let json = snapshot.to_json().expect("snapshot json");
                    let parsed = WorkflowState::from_json(&json).expect("snapshot parse");
                    assert_eq!(parsed, snapshot);
                    let fresh = WorkflowEngine::new(definition.clone());
                    let restored = fresh.restore(parsed).expect("snapshot must be consistent");
                    assert_eq!(
                        restored.to_json().expect("restored json"),
                        json,
                        "snapshot did not re-serialize identically"
                    );
                    // Serialize/digest readers must not tear either.
                    engine.serialize(None).expect("serialize");
                    engine.state_digest(None).expect("digest");
                    snapshots.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    writer.join().expect("writer");
    for reader in readers {
        reader.join().expect("reader");
    }
    assert!(snapshots.load(Ordering::Relaxed) > 0);
    let finished = engine.state().expect("state");
    assert_eq!(finished.status(), WorkflowStatus::Completed);
    assert_eq!(finished.history().len(), 24);
    assert_eq!(finished.current_state().to_string_lossy(), "s24");
}

#[test]
fn failing_sink_after_store_matches_python_mutation_order() {
    struct FailOnce {
        event_type: &'static str,
        fired: AtomicBool,
    }
    impl WorkflowEventSink for FailOnce {
        fn emit(&self, event: &WorkflowEvent) -> Result<(), kala_kernel::WorkflowError> {
            if event.event_type.to_string_lossy() == self.event_type
                && !self.fired.swap(true, Ordering::SeqCst)
            {
                return Err(kala_kernel::WorkflowError::Callback(
                    "journal rejected".to_owned(),
                ));
            }
            Ok(())
        }
    }

    // start stores then emits: a callback error leaves the created state.
    let engine =
        WorkflowEngine::new(definition(&simple_definition(false))).with_sink(Arc::new(FailOnce {
            event_type: "workflow.created",
            fired: AtomicBool::new(false),
        }));
    assert!(engine.start(&Text::new("workflow-1"), None).is_err());
    assert!(engine.state().is_some());

    // run stores only after emitting: a callback error leaves Created.
    let engine =
        WorkflowEngine::new(definition(&simple_definition(false))).with_sink(Arc::new(FailOnce {
            event_type: "workflow.transition",
            fired: AtomicBool::new(false),
        }));
    engine.start(&Text::new("workflow-1"), None).expect("start");
    assert!(engine.run(None, None, None, None).is_err());
    assert_eq!(
        engine.state().expect("state").status(),
        WorkflowStatus::Created
    );
}
