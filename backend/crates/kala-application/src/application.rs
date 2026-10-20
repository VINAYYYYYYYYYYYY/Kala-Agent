//! The local composition root.
//!
//! This module is the only place that knows how the field-independent kernel,
//! the persistence layer, the bounded tool plane, the provider-neutral model
//! gateway and the declarative domain packages are wired together. It owns
//! composition and lifecycle, nothing else.
//!
//! The root is local and offline by construction: it builds a deterministic
//! in-process model client, grants only the capabilities the declared domain
//! packages need, and never opens a socket. A gateway that needs egress is
//! reported as such, and only a gateway the caller supplies can make the
//! process reach a hosted model.
//!
//! A design revision is written in two phases: preparation derives the commit,
//! reduces it to a state digest and an action digest, stores the report bytes
//! content-addressed and opens a pre-commit checkpoint bound to both digests;
//! the commit resolves that checkpoint against the digests recomputed from the
//! state that exists at that moment and writes under a transactional head
//! compare-and-swap.
//!
//! Kernel policy and budget state live in reserved snapshots and are restored
//! at startup; durable state that is missing, unverifiable or invalid stops the
//! launch rather than restoring defaults.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use kala_api::config::{Settings, settings_from_env};
use kala_artifacts::{ContentAddressedArtifactStore, DEFAULT_MAX_OBJECT_SIZE};
use kala_budget::{BudgetLedger, Counter, RunBudget};
use kala_contracts::{
    ApprovalKind, ApprovalRequest, ApprovalStatus, ArtifactClassification, ArtifactRef,
    ContractModel, DesignRevision, DesignSpec, DomainPackageManifest, EvaluationResult, Provenance,
    SideEffectLevel, Timestamp, WorkflowDefinition, WorkflowStatus,
};
use kala_domains::{DomainParameters, NON_AUTHORITATIVE_FINDING};
use kala_json::{CanonicalValue, JsonText};
use kala_kernel::policy::{
    DecisionContext, PolicyDecision, PolicyDecisionStatus, PolicyEngine, PolicyError, PolicyReason,
    PolicyRequest,
};
use kala_kernel::workflow::{
    CheckpointContext, StepAttempt, WorkflowApproval, WorkflowEngine, WorkflowState,
    WorkflowTransitionRecord,
};
use kala_persistence::{
    AppendOptions, ChainVerification, Database, Event, EventFilter, EventJournal, PersistenceError,
    SnapshotStore, WorkflowSnapshot, parse_json_object, persistence_limits,
};
use kala_repositories::{
    ApprovalRequestRepository, DesignRevisionRepository, DesignSpecRepository, RepositoryError,
};
use kala_runtime::agent::{CatalogStep as AgentCatalogStep, OfflinePlanner, Planner, StepCatalog};
use kala_runtime::gateway::{
    GatewayTimestamp, LOCAL_PROVIDER_ID, LocalModelClient, ModelCapability, ModelGateway,
};
use kala_tools::executor::{ExecutorConfig, SandboxProfile, ToolExecutor};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    SIMULATOR_CAPABILITIES, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION, register_simulator,
};
use num_bigint::BigInt;

use crate::domains::DomainBinding;
use crate::dto::{
    BudgetUsageView, CatalogStepView, CheckpointView, DomainDetail, DomainEvaluationReport,
    DomainPackageView, EventPage, EventView, MechanicalRunResult, RevisionCommitView,
    RevisionRecord, StepAttemptView, WorkflowDefinitionView, WorkflowStateView,
    WorkflowTransitionRecordView, WorkflowTransitionView,
};
use crate::error::{ApplicationError, ApplicationResult, ErrorKind};
use crate::value::{
    AppText, MAX_IDENTIFIER_LENGTH, canonical_digest, canonical_dumps, chrono_from_contract,
    contract_timestamp, field, identifier_sequence, json_text_utf8, kernel_from_chrono,
    optional_text, optional_text_value, require_digest, require_identifier, require_limit,
    require_stored_digest, require_stored_identifier, require_stored_sequence, require_text,
    sha256_hex_text, text_ref, text_string,
};

/// The local operator identity.
pub const LOCAL_ACTOR_ID: &str = "local-user";
/// The three declared domain names.
pub const DOMAIN_NAMES: [&str; 3] = ["building", "mechanical", "wing"];
/// The mechanical domain name.
pub const MECHANICAL_DOMAIN: &str = "mechanical";
/// The default mechanical design identifier.
pub const DEFAULT_MECHANICAL_DESIGN_ID: &str = "mechanical-reference";
/// The capabilities a revision commit requires.
pub const PRE_COMMIT_CAPABILITIES: [&str; 2] = ["artifact.write", "design.write"];
/// The options of a checkpoint approval.
pub const APPROVAL_OPTIONS: [&str; 2] = ["approve", "reject"];
/// The accepted checkpoint decisions.
pub const CHECKPOINT_DECISIONS: [&str; 2] = ["approve", "reject"];
/// The accepted revision-commit decisions.
pub const REVISION_DECISIONS: [&str; 2] = ["approve", "reject"];
/// The error code a failed hard constraint is recorded with.
pub const HARD_CONSTRAINT_CODE: &str = "hard_constraint_violation";
/// The planner objective of the mechanical reference run.
pub const MECHANICAL_PLAN_OBJECTIVE: &str =
    "section plate material pattern load stress margin safety";
/// The default checkpoint rejection note.
pub const DEFAULT_REJECT_REASON: &str = "checkpoint rejected by the local operator";
/// Design identifier bound.
pub const MAX_DESIGN_ID_LENGTH: usize = 64;
/// Summary bound.
pub const MAX_SUMMARY_LENGTH: usize = 512;
/// Event payload bound used by the read view.
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 65_536;
/// Spec attribute bound.
pub const MAX_SPEC_ATTRIBUTES_BYTES: usize = 8_192;
/// Report bound.
pub const MAX_REPORT_BYTES: usize = 524_288;
/// Reference step bound.
pub const MAX_REFERENCE_STEPS: usize = 64;
/// Parameter entry bound.
pub const MAX_ATTRIBUTE_COUNT: usize = 4_096;
/// Revision list bound.
pub const MAX_REVISION_LIMIT: i64 = 1_000;
/// Event page bound.
pub const MAX_EVENT_LIMIT: i64 = 1_000;
/// The report media type.
pub const REPORT_MEDIA_TYPE: &str = "application/json";
/// The report classification.
pub const REPORT_CLASSIFICATION: ArtifactClassification = ArtifactClassification::Text;
/// The artifact URI scheme prefix.
pub const ARTIFACT_SCHEME_PREFIX: &str = "artifact://sha256/";
/// The workflow identifier prefix.
pub const WORKFLOW_ID_PREFIX: &str = "workflow-";
/// The revision commit operation name.
pub const REVISION_COMMIT_OPERATION: &str = "design-revision-commit";
/// The revision commit checkpoint kind.
pub const REVISION_COMMIT_CHECKPOINT: ApprovalKind = ApprovalKind::PreCommit;
/// The revision commit request prefix.
pub const REVISION_COMMIT_PREFIX: &str = "commit-";
/// The digest prefix length used in a request identity.
pub const DIGEST_PREFIX_LENGTH: usize = 16;
/// The reserved policy-state aggregate.
pub const POLICY_STATE_AGGREGATE: &str = "kernel-policy-state";
/// The reserved budget-state aggregate.
pub const BUDGET_STATE_AGGREGATE: &str = "kernel-budget-state";
/// The reserved kernel-state namespace.
pub const KERNEL_STATE_NAMESPACE: &str = "kernel-state:";
/// The prepared revision event.
pub const PREPARED_REVISION_EVENT: &str = "revision.prepared";
/// The decided revision event.
pub const DECIDED_REVISION_EVENT: &str = "revision.commit_decided";
/// The committed revision event.
pub const COMMITTED_REVISION_EVENT: &str = "revision.created";
/// The rejected revision event.
pub const REJECTED_REVISION_EVENT: &str = "revision.commit_rejected";
/// The conflicted revision event.
pub const CONFLICTED_REVISION_EVENT: &str = "revision.commit_conflicted";
/// The local deterministic model id fallback.
pub const DEFAULT_LOCAL_MODEL_ID: &str = "local-deterministic";
/// The reference evaluation transformation.
pub const TRANSFORM_REFERENCE_EVALUATION: &str = "reference-workflow-evaluation";
/// The mechanical run transformation.
pub const TRANSFORM_MECHANICAL_RUN: &str = "mechanical-reference-workflow";

/// The reserved kernel-state aggregate names.
#[must_use]
pub fn reserved_workflow_ids() -> [&'static str; 2] {
    [POLICY_STATE_AGGREGATE, BUDGET_STATE_AGGREGATE]
}

/// The policy reasons on which a revision or checkpoint decision is refused
/// before the decision is applied.
fn is_undecidable(reason: PolicyReason) -> bool {
    matches!(
        reason,
        PolicyReason::Aborted
            | PolicyReason::CapabilityExpired
            | PolicyReason::CapabilityNotGranted
            | PolicyReason::InvalidRequest
            | PolicyReason::Rejected
            | PolicyReason::ScopeMismatch
    )
}

/// The queue of workflow events the engine emitted during one call.
type WorkflowEventQueue = Arc<std::sync::Mutex<Vec<(String, String, CanonicalValue)>>>;

/// An injectable application clock.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync + 'static>;

/// Components a caller may supply instead of letting the root build them.
///
/// A supplied component belongs to the caller: [`KalaApplication::close`] never
/// closes it.
#[derive(Default)]
pub struct Components {
    /// The persistence database, shared with the journal unless it is supplied
    /// separately.
    pub database: Option<Arc<Database>>,
    /// The database the journal reads and writes, when it is not the
    /// application database.
    pub journal_database: Option<Arc<Database>>,
    /// The artifact store.
    pub artifact_store: Option<ContentAddressedArtifactStore>,
    /// The budget ledger.
    pub budget: Option<Arc<BudgetLedger>>,
    /// The policy engine.
    pub policy: Option<Arc<PolicyEngine>>,
    /// The tool registry.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// The tool executor.
    pub tool_executor: Option<Arc<ToolExecutor>>,
    /// The model gateway.
    pub model_gateway: Option<Arc<ModelGateway>>,
    /// The deterministic planner.
    pub planner: Option<OfflinePlanner>,
    /// The registered domain bindings.
    pub domains: Option<Vec<DomainBinding>>,
    /// The application clock.
    pub clock: Option<Clock>,
}

/// One fully derived, not yet written revision commit.
#[derive(Clone, Debug)]
struct DerivedCommit {
    design_id: String,
    domain: String,
    revision_id: String,
    provenance_id: String,
    sequence: BigInt,
    parent_revision_id: Option<String>,
    expected_head_revision_id: Option<String>,
    spec: DesignSpec,
    artifact: ArtifactRef,
    domain_package: DomainPackageManifest,
    provenance_inputs: Provenance,
    parameters: CanonicalValue,
    spec_digest: String,
    parameters_digest: String,
    package_digests: Vec<String>,
    parent_provenance_ids: Vec<String>,
    transformation_ids: Vec<String>,
    workflow_id: Option<String>,
    state_digest: String,
    action_digest: String,
    request_id: String,
}

/// The local composition root.
pub struct KalaApplication {
    settings: Settings,
    clock: Clock,
    closed: AtomicBool,
    database: Arc<Database>,
    owns_database: bool,
    journal_database: Arc<Database>,
    artifact_store: ContentAddressedArtifactStore,
    budget: Arc<BudgetLedger>,
    owns_budget: bool,
    policy: Arc<PolicyEngine>,
    owns_policy: bool,
    registry: Arc<ToolRegistry>,
    executor: Arc<ToolExecutor>,
    gateway: Arc<ModelGateway>,
    owns_gateway: bool,
    catalog: Arc<StepCatalog>,
    planner: OfflinePlanner,
    domains: Vec<DomainBinding>,
}

impl std::fmt::Debug for KalaApplication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KalaApplication")
            .field("data_dir", &self.settings.data_dir)
            .field("closed", &self.closed())
            .field("provider_id", &self.provider_id())
            .field("model_id", &self.model_id())
            .finish()
    }
}

impl KalaApplication {
    /// Open an application with default settings from the environment.
    ///
    /// # Errors
    ///
    /// Fails when the environment settings are invalid, the durable state
    /// cannot be verified or restored, or a component cannot be built.
    pub fn open_default() -> ApplicationResult<Self> {
        let settings = settings_from_env(&std::collections::BTreeMap::new()).map_err(|errors| {
            let message = errors
                .iter()
                .map(|error| format!("{}: {}", error.location, error.message))
                .collect::<Vec<_>>()
                .join("; ");
            ApplicationError::invalid_parameters(format!(
                "the local settings are invalid: {message}"
            ))
        })?;
        Self::open(settings)
    }

    /// Open an application over the supplied settings.
    ///
    /// # Errors
    ///
    /// Fails when the durable state cannot be verified or restored, or a
    /// component cannot be built.
    pub fn open(settings: Settings) -> ApplicationResult<Self> {
        Self::with_components(settings, Components::default())
    }

    /// Open an application, supplying some or all components.
    ///
    /// # Errors
    ///
    /// Fails when the durable state cannot be verified or restored, or a
    /// component cannot be built.
    pub fn with_components(settings: Settings, components: Components) -> ApplicationResult<Self> {
        let clock: Clock = components
            .clock
            .clone()
            .unwrap_or_else(|| Arc::new(Utc::now));

        let (database, owns_database) = match components.database {
            Some(database) => (database, false),
            None => (
                Arc::new(Database::open(settings.database_path()).map_err(|error| {
                    ApplicationError::application(format!(
                        "the local database cannot be opened: {error}"
                    ))
                })?),
                true,
            ),
        };
        let journal_database = match components.journal_database {
            Some(database) => database,
            None => Arc::clone(&database),
        };
        // Python constructs the journal here, and construction verifies the
        // whole stored chain. Keep that failure at the same point in startup.
        EventJournal::new(&journal_database).map_err(|error| {
            ApplicationError::application(format!("the event journal cannot be verified: {error}"))
        })?;

        let artifact_store = match components.artifact_store {
            Some(store) => store,
            None => {
                ContentAddressedArtifactStore::new(settings.artifact_dir(), DEFAULT_MAX_OBJECT_SIZE)
                    .map_err(|error| {
                        ApplicationError::application(format!(
                            "the artifact store cannot be opened: {error}"
                        ))
                    })?
            }
        };

        let (budget, owns_budget) = match components.budget {
            Some(budget) => (budget, false),
            None => (Arc::new(BudgetLedger::new(RunBudget::new())), true),
        };
        let (policy, owns_policy) = match components.policy {
            Some(policy) => (policy, false),
            None => (
                Arc::new(
                    PolicyEngine::from_capabilities(default_capabilities())
                        .map_err(|error| ApplicationError::application(error.to_string()))?,
                ),
                true,
            ),
        };

        let registry = match components.tool_registry {
            Some(registry) => registry,
            None => {
                let registry = Arc::new(ToolRegistry::new());
                register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
                    .map_err(|error| ApplicationError::application(error.to_string()))?;
                registry
            }
        };

        let executor = match components.tool_executor {
            Some(executor) => executor,
            None => {
                let profile = SandboxProfile::new()
                    .with_max_workers(usize::from(settings.worker_count))
                    .with_default_timeout_seconds(settings.tool_timeout_seconds.max(1.0));
                let mut granted = BTreeSet::new();
                for capability in SIMULATOR_CAPABILITIES {
                    granted.insert(capability.to_owned());
                }
                let config = ExecutorConfig {
                    registry: Arc::clone(&registry),
                    run_directory: settings.data_dir.join("runs"),
                    granted_capabilities: granted,
                    profile,
                    ledger: Some(Arc::clone(&budget)),
                    policy: Some(Arc::clone(&policy)),
                    ..ExecutorConfig::new(Arc::clone(&registry), settings.data_dir.join("runs"))
                };
                Arc::new(ToolExecutor::new(config).map_err(|error| {
                    ApplicationError::application(format!(
                        "the bounded tool executor cannot be built: {error}"
                    ))
                })?)
            }
        };

        let model_id = if crate::value::is_safe_identifier(&settings.model_id) {
            settings.model_id.clone()
        } else {
            DEFAULT_LOCAL_MODEL_ID.to_owned()
        };
        let gateway_clock: Arc<dyn Fn() -> GatewayTimestamp + Send + Sync> = {
            let clock = Arc::clone(&clock);
            Arc::new(move || GatewayTimestamp::Aware(kernel_from_chrono(clock())))
        };
        let (gateway, owns_gateway) = match components.model_gateway {
            Some(gateway) => (gateway, false),
            None => {
                let client = LocalModelClient::new(
                    &model_id,
                    LOCAL_PROVIDER_ID,
                    vec![ModelCapability::StructuredOutput, ModelCapability::Seed],
                )
                .map_err(|error| ApplicationError::application(error.to_string()))?;
                (
                    Arc::new(
                        ModelGateway::new(Arc::new(client))
                            .with_ledger(Arc::clone(&budget))
                            .with_policy(Arc::clone(&policy))
                            .with_clock(gateway_clock),
                    ),
                    true,
                )
            }
        };

        let catalog = Arc::new(mechanical_step_catalog()?);
        let planner = match components.planner {
            Some(planner) => planner,
            None => OfflinePlanner::new(
                Arc::clone(&catalog),
                Some(kala_domains::mechanical::REFERENCE_WORKFLOW_STEP_IDS[0].as_str()),
            )
            .map_err(|error| ApplicationError::application(error.to_string()))?,
        };

        let domains = match components.domains {
            Some(domains) => domains,
            None => DomainBinding::defaults(),
        };
        if domains.is_empty() {
            return Err(ApplicationError::invalid_parameters(
                "at least one domain package must be registered",
            ));
        }
        let mut seen = BTreeSet::new();
        for binding in &domains {
            if !seen.insert(binding.domain()) {
                return Err(ApplicationError::invalid_parameters(format!(
                    "domain {} is registered twice",
                    binding.domain().as_str()
                )));
            }
        }

        let application = Self {
            settings,
            clock,
            closed: AtomicBool::new(false),
            database,
            owns_database,
            journal_database,
            artifact_store,
            budget,
            owns_budget,
            policy,
            owns_policy,
            registry,
            executor,
            gateway,
            owns_gateway,
            catalog,
            planner,
            domains,
        };
        if application.owns_policy || application.owns_budget {
            application.restore_kernel_state()?;
        }
        Ok(application)
    }

    // -----------------------------------------------------------------
    // Component accessors
    // -----------------------------------------------------------------

    /// The validated settings this application was built from.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The open persistence database.
    #[must_use]
    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    /// The append-only event journal, bound to the shared database.
    ///
    /// # Errors
    ///
    /// Fails closed when the stored chain cannot be verified.
    pub fn journal(&self) -> ApplicationResult<EventJournal<'_>> {
        EventJournal::new(&self.journal_database).map_err(journal_error)
    }

    /// Verify the complete stored journal chain.
    ///
    /// # Errors
    ///
    /// Fails closed when any stored row is unverifiable.
    pub fn verify_chain(&self) -> ApplicationResult<ChainVerification> {
        EventJournal::new(&self.journal_database)
            .and_then(|journal| journal.verify_chain())
            .map_err(|error| {
                ApplicationError::application(format!(
                    "the event journal cannot be verified: {error}"
                ))
            })
    }

    /// The content-addressed artifact store.
    #[must_use]
    pub fn artifact_store(&self) -> &ContentAddressedArtifactStore {
        &self.artifact_store
    }

    /// The workflow snapshot store.
    #[must_use]
    pub fn snapshots(&self) -> SnapshotStore<'_> {
        SnapshotStore::new(&self.database)
    }

    /// The design specification repository.
    #[must_use]
    pub fn design_specs(&self) -> DesignSpecRepository<'_> {
        DesignSpecRepository::new(&self.database)
    }

    /// The immutable design revision repository.
    #[must_use]
    pub fn design_revisions(&self) -> DesignRevisionRepository<'_> {
        DesignRevisionRepository::new(&self.database)
    }

    /// The policy checkpoint repository.
    #[must_use]
    pub fn approval_requests(&self) -> ApprovalRequestRepository<'_> {
        ApprovalRequestRepository::new(&self.database)
    }

    /// The shared budget ledger.
    #[must_use]
    pub fn budget(&self) -> &Arc<BudgetLedger> {
        &self.budget
    }

    /// The shared capability policy engine.
    #[must_use]
    pub fn policy(&self) -> &Arc<PolicyEngine> {
        &self.policy
    }

    /// The tool registry holding the declared reference tools.
    #[must_use]
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    /// The bounded tool executor.
    #[must_use]
    pub fn tool_executor(&self) -> &Arc<ToolExecutor> {
        &self.executor
    }

    /// The provider-neutral model gateway.
    #[must_use]
    pub fn model_gateway(&self) -> &Arc<ModelGateway> {
        &self.gateway
    }

    /// The deterministic offline planner.
    #[must_use]
    pub fn planner(&self) -> &OfflinePlanner {
        &self.planner
    }

    /// The mechanical step catalog the planner selects from.
    #[must_use]
    pub fn step_catalog(&self) -> &Arc<StepCatalog> {
        &self.catalog
    }

    /// The registered domain names in a stable order.
    #[must_use]
    pub fn domains(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .domains
            .iter()
            .map(|binding| binding.domain().as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// Whether this application has released its own resources.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Whether the configured model gateway can reach the network.
    #[must_use]
    pub fn offline(&self) -> bool {
        !self.gateway.requires_egress()
    }

    /// Whether the wired model client can reach a hosted provider.
    ///
    /// This is a fact about the client this application actually holds, never
    /// about a setting.
    #[must_use]
    pub fn egress_configured(&self) -> bool {
        self.gateway.requires_egress()
    }

    /// Whether the settings declare a provider-neutral model endpoint.
    ///
    /// A declared endpoint is configuration for a future adapter, not a wired
    /// one, and is never reported as egress.
    #[must_use]
    pub fn provider_endpoint_declared(&self) -> bool {
        self.settings.model_endpoint.is_some()
    }

    /// Whether the model gateway was constructed by the caller.
    #[must_use]
    pub fn model_gateway_supplied(&self) -> bool {
        !self.owns_gateway
    }

    /// The identity of the configured model provider.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        self.gateway.provider_id()
    }

    /// The identity of the configured model.
    #[must_use]
    pub fn model_id(&self) -> &str {
        self.gateway.model_id()
    }

    // -----------------------------------------------------------------
    // Domain declarations
    // -----------------------------------------------------------------

    /// Return every registered domain package declaration.
    #[must_use]
    pub fn list_domain_packages(&self) -> Vec<DomainPackageView> {
        self.sorted_bindings()
            .into_iter()
            .map(package_view)
            .collect()
    }

    /// Return every registered workflow definition declaration.
    #[must_use]
    pub fn list_workflow_definitions(&self) -> Vec<WorkflowDefinitionView> {
        self.sorted_bindings()
            .into_iter()
            .map(workflow_view)
            .collect()
    }

    /// Return the declared catalog steps of one domain package.
    ///
    /// # Errors
    ///
    /// Fails when the domain is not registered.
    pub fn list_catalog_steps(&self, domain: &str) -> ApplicationResult<Vec<CatalogStepView>> {
        self.describe_domain(domain)
            .map(|detail| detail.catalog_steps)
    }

    /// Return one domain package with its workflow and catalog steps.
    ///
    /// # Errors
    ///
    /// Fails when the domain is not registered.
    pub fn describe_domain(&self, domain: &str) -> ApplicationResult<DomainDetail> {
        let binding = self.binding(domain)?;
        let definition_id = binding.package().workflow().definition_id();
        let workflow = self
            .list_workflow_definitions()
            .into_iter()
            .find(|item| item.definition_id == definition_id)
            .ok_or_else(|| {
                ApplicationError::application("the declared workflow definition is not registered")
            })?;
        let package = package_view(binding);
        let catalog_steps = binding
            .package()
            .steps()
            .iter()
            .map(|step| CatalogStepView {
                step_id: step.step_id().to_owned(),
                name: step.name().to_owned(),
                description: step.description().to_owned(),
                input_names: step.input_names().to_vec(),
                metric_names: step.metric_names().to_vec(),
                required_capabilities: step.required_capabilities().to_vec(),
                checkpoint_kind: step.checkpoint_kind(),
                max_attempts: step.max_attempts(),
            })
            .collect();
        Ok(DomainDetail {
            package,
            workflow,
            catalog_steps,
        })
    }

    // -----------------------------------------------------------------
    // Evaluation
    // -----------------------------------------------------------------

    /// Evaluate one reference design of a domain package deterministically.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, the domain is unknown, the design
    /// identifier is unsafe, or the parameters are invalid.
    pub fn evaluate_domain_design(
        &self,
        domain: &str,
        parameters: Option<&CanonicalValue>,
        design_id: Option<&str>,
    ) -> ApplicationResult<DomainEvaluationReport> {
        self.require_open()?;
        let binding = self.binding(domain)?;
        let subject_id = match design_id {
            Some(design_id) => require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)?,
            None => binding.subject_id().to_owned(),
        };
        let model = binding.parameters(parameters)?;
        let report = binding.evaluate(&subject_id, &model)?;
        self.unit_of_work(|application| {
            let payload = CanonicalValue::object([
                (
                    "blocked_step_id",
                    report
                        .blocked_step_id
                        .as_ref()
                        .map_or(CanonicalValue::Null, |value| {
                            CanonicalValue::text(value.as_str())
                        }),
                ),
                (
                    "completed_step_ids",
                    CanonicalValue::array(
                        report
                            .completed_step_ids
                            .iter()
                            .map(|value| CanonicalValue::text(value.as_app_str())),
                    ),
                ),
                ("domain", CanonicalValue::text(report.domain.as_str())),
                (
                    "evaluation_count",
                    CanonicalValue::integer(BigInt::from(report.evaluations.len())),
                ),
                (
                    "package_digest",
                    CanonicalValue::text(report.package_digest.as_str()),
                ),
                (
                    "parameters_digest",
                    CanonicalValue::text(report.parameters_digest.as_str()),
                ),
                ("passed", CanonicalValue::Bool(report.passed)),
                ("status", CanonicalValue::text(report.status.as_str())),
            ]);
            application.append_event("domain.evaluated", &subject_id, &payload, None)?;
            Ok(())
        })?;
        Ok(report)
    }

    // -----------------------------------------------------------------
    // Revisions
    // -----------------------------------------------------------------

    /// Prepare and commit one revision in a single local operator call.
    ///
    /// # Errors
    ///
    /// Fails and writes nothing when the commit is refused, the capability is
    /// revoked, the design head moved, or the parent does not extend the head.
    #[allow(clippy::too_many_arguments)]
    pub fn create_design_revision(
        &self,
        design_id: &str,
        domain: &str,
        summary: Option<&str>,
        parameters: Option<&CanonicalValue>,
        parent_revision_id: Option<&str>,
        parent_provenance_ids: &[String],
        transformation_ids: &[String],
        workflow_id: Option<&str>,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
    ) -> ApplicationResult<RevisionRecord> {
        self.require_open()?;
        let prepared = self.prepare_design_revision(
            design_id,
            domain,
            summary,
            parameters,
            parent_revision_id,
            parent_provenance_ids,
            transformation_ids,
            workflow_id,
        )?;
        self.commit_design_revision(
            &prepared.request_id,
            actor_id,
            state_digest,
            action_digest,
            None,
            None,
        )
    }

    /// Derive one revision commit and open its pre-commit policy checkpoint.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, an identifier is unsafe, the
    /// domain is unknown, parameters are invalid, the parent is not the head,
    /// or the request already carries a decision.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_design_revision(
        &self,
        design_id: &str,
        domain: &str,
        summary: Option<&str>,
        parameters: Option<&CanonicalValue>,
        parent_revision_id: Option<&str>,
        parent_provenance_ids: &[String],
        transformation_ids: &[String],
        workflow_id: Option<&str>,
    ) -> ApplicationResult<RevisionCommitView> {
        self.require_open()?;
        let subject = require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)?;
        let binding = self.binding(domain)?;
        let workflow = match workflow_id {
            None => None,
            Some(workflow_id) => Some(require_workflow_identifier(workflow_id)?),
        };
        let derived = self.derive_commit(
            binding,
            &subject,
            summary,
            parameters,
            parent_revision_id,
            parent_provenance_ids,
            transformation_ids,
            workflow.clone(),
            true,
        )?;
        let request = commit_policy_request(&derived)?;
        let created_at = RefCell::new(None);
        let payload = self.unit_of_work(|application| {
            application.require_undecided_commit(&derived.request_id)?;
            let mut record = application
                .policy
                .get_approval(&text(&derived.request_id), None, None)
                .map_err(policy_application_error)?;
            if record.is_none() {
                record = Some(
                    application
                        .policy
                        .request_approval(&request, None)
                        .map_err(policy_application_error)?,
                );
            }
            let record = record.expect("requested");
            if record.status() != ApprovalStatus::Pending {
                return Err(ApplicationError::checkpoint_not_pending(format!(
                    "revision commit {} is already {}",
                    derived.request_id,
                    record.status().as_str()
                )));
            }
            let created = application.now()?;
            *created_at.borrow_mut() = Some(created);
            application.save_commit_approval(&derived, &request, created)?;
            let payload = application
                .prepared_payload(&derived, contract_timestamp(kernel_from_chrono(created)))?;
            application.append_event(
                PREPARED_REVISION_EVENT,
                &derived.request_id,
                &payload,
                derived.workflow_id.as_deref(),
            )?;
            Ok(payload)
        })?;
        let created_at = created_at
            .into_inner()
            .expect("the unit of work records the preparation time");
        self.commit_view_from_prepared(
            &payload,
            &derived.request_id,
            ApprovalStatus::Pending,
            &contract_timestamp(kernel_from_chrono(created_at)),
            None,
            None,
            None,
            false,
        )
    }

    /// Record an approve or reject decision for one prepared revision commit.
    ///
    /// # Errors
    ///
    /// Fails and writes nothing when the decision is invalid, the request is
    /// unknown or already decided, the supplied digests are stale, the policy
    /// refuses the decision, or the design head moved.
    #[allow(clippy::too_many_arguments)]
    pub fn decide_revision_commit(
        &self,
        request_id: &str,
        decision: &str,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
        reason: Option<&str>,
        workflow_id: Option<&str>,
    ) -> ApplicationResult<RevisionCommitView> {
        self.require_open()?;
        let identifier = require_identifier(request_id, "request_id", MAX_IDENTIFIER_LENGTH)?;
        let normalized = require_identifier(decision, "decision", MAX_IDENTIFIER_LENGTH)?;
        if !REVISION_DECISIONS.contains(&normalized.as_str()) {
            return Err(ApplicationError::invalid_parameters(format!(
                "decision must be one of {}",
                REVISION_DECISIONS.join(", ")
            )));
        }
        let mut actor = require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)?;
        let prepared = self.prepared_commit(&identifier)?;
        self.require_undecided_commit(&identifier)?;
        let derived = self.rederive_commit(&prepared)?;
        if let Some(state_digest) = state_digest {
            if state_digest != derived.state_digest {
                return Err(ApplicationError::checkpoint_digest_mismatch(
                    "the supplied state digest is stale",
                ));
            }
        }
        if let Some(action_digest) = action_digest {
            if action_digest != derived.action_digest {
                return Err(ApplicationError::checkpoint_digest_mismatch(
                    "the supplied action digest is stale",
                ));
            }
        }
        if let Some(workflow_id) = workflow_id {
            actor =
                self.workflow_commit_actor(&require_workflow_identifier(workflow_id)?, &derived)?;
        }
        let request = commit_policy_request(&derived)?;
        let created_at = self.prepared_created_at(&identifier)?;
        let decision_note = optional_text(reason, "reason", MAX_SUMMARY_LENGTH)?;
        let outcome = self.unit_of_work(|application| {
            application.require_undecided_commit(&identifier)?;
            application
                .policy
                .request_approval(&request, None)
                .map_err(policy_application_error)?;
            let precheck = application
                .policy
                .decide(&request, None)
                .map_err(policy_application_error)?;
            if is_undecidable(precheck.reason()) {
                return Err(ApplicationError::checkpoint_decision_denied(format!(
                    "the policy engine will not accept a revision commit decision: {}",
                    precheck.reason().as_str()
                )));
            }
            let (status, selected_option, note) = if normalized == "approve" {
                application.approve_policy(
                    &request,
                    (&derived.state_digest, &derived.action_digest),
                    &actor,
                )?;
                (ApprovalStatus::Approved, "approve", decision_note.clone())
            } else {
                application.reject_policy(
                    &request,
                    (&derived.state_digest, &derived.action_digest),
                    &actor,
                    decision_note.as_deref(),
                )?;
                (
                    ApprovalStatus::Rejected,
                    "reject",
                    Some(
                        decision_note
                            .clone()
                            .unwrap_or_else(|| DEFAULT_REJECT_REASON.to_owned()),
                    ),
                )
            };
            let decided_at = std::cmp::max(application.now()?, chrono_from_contract(&created_at));
            application.save_decided_commit_approval(
                &derived,
                &request,
                status,
                selected_option,
                &actor,
                &created_at,
                &contract_timestamp(kernel_from_chrono(decided_at)),
            )?;
            application.require_stored_decision(&derived.request_id, status, &actor)?;
            let decided_payload = CanonicalValue::object([
                (
                    "action_digest",
                    CanonicalValue::text(derived.action_digest.as_str()),
                ),
                ("actor_id", CanonicalValue::text(actor.as_str())),
                (
                    "design_id",
                    CanonicalValue::text(derived.design_id.as_str()),
                ),
                (
                    "revision_id",
                    CanonicalValue::text(derived.revision_id.as_str()),
                ),
                ("selected_option", CanonicalValue::text(selected_option)),
                (
                    "state_digest",
                    CanonicalValue::text(derived.state_digest.as_str()),
                ),
                ("status", CanonicalValue::text(status.as_str())),
                (
                    "workflow_id",
                    derived
                        .workflow_id
                        .as_ref()
                        .map_or(CanonicalValue::Null, |value| {
                            CanonicalValue::text(value.as_str())
                        }),
                ),
            ]);
            application.append_event(
                DECIDED_REVISION_EVENT,
                &derived.request_id,
                &decided_payload,
                derived.workflow_id.as_deref(),
            )?;
            let mut committed = false;
            if status == ApprovalStatus::Approved {
                committed = application.write_revision(&derived, &actor, note.as_deref())?;
            } else {
                let rejected_payload = CanonicalValue::object([
                    ("actor_id", CanonicalValue::text(actor.as_str())),
                    (
                        "design_id",
                        CanonicalValue::text(derived.design_id.as_str()),
                    ),
                    (
                        "reason",
                        note.as_ref().map_or(CanonicalValue::Null, |value| {
                            CanonicalValue::text(value.as_str())
                        }),
                    ),
                    (
                        "revision_id",
                        CanonicalValue::text(derived.revision_id.as_str()),
                    ),
                ]);
                application.append_event(
                    REJECTED_REVISION_EVENT,
                    &derived.request_id,
                    &rejected_payload,
                    derived.workflow_id.as_deref(),
                )?;
            }
            Ok((status, note, committed, decided_at))
        })?;
        let (status, note, committed, decided_at) = outcome;
        if status == ApprovalStatus::Approved && !committed {
            return Err(ApplicationError::revision_head_moved(format!(
                "the head of design {} moved before the revision was written",
                derived.design_id
            )));
        }
        self.commit_view_from_prepared(
            &prepared,
            &derived.request_id,
            status,
            &created_at,
            Some(&actor),
            Some(&contract_timestamp(kernel_from_chrono(decided_at))),
            note.as_deref(),
            committed,
        )
    }

    /// Approve one prepared revision commit and return the stored revision.
    ///
    /// # Errors
    ///
    /// Fails when the commit is not approved, unknown, or already decided.
    pub fn commit_design_revision(
        &self,
        request_id: &str,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
        reason: Option<&str>,
        workflow_id: Option<&str>,
    ) -> ApplicationResult<RevisionRecord> {
        let decided = self.decide_revision_commit(
            request_id,
            "approve",
            actor_id,
            state_digest,
            action_digest,
            reason,
            workflow_id,
        )?;
        if !decided.committed {
            return Err(ApplicationError::revision_commit_rejected(format!(
                "revision commit {} was not approved",
                decided.request_id
            )));
        }
        self.get_design_revision(&decided.revision_id)
    }

    /// Return one prepared or decided revision commit as it is stored.
    ///
    /// # Errors
    ///
    /// Fails when the request identifier is unsafe or the preparation is
    /// unknown.
    pub fn get_revision_commit(&self, request_id: &str) -> ApplicationResult<RevisionCommitView> {
        self.require_open()?;
        let identifier = require_identifier(request_id, "request_id", MAX_IDENTIFIER_LENGTH)?;
        let prepared = self.prepared_commit(&identifier)?;
        let stored = ApprovalRequestRepository::new(&self.database)
            .get(&identifier)
            .map_err(repository_error)?;
        let decided = self
            .policy
            .get_approval(&text(&identifier), None, None)
            .map_err(policy_application_error)?;
        let status = stored
            .as_ref()
            .map_or(ApprovalStatus::Pending, ApprovalRequest::status);
        let created_at = self.prepared_created_at(&identifier)?;
        let stored_decided_at = stored
            .as_ref()
            .and_then(ApprovalRequest::decided_at)
            .cloned();
        let committed = stored
            .as_ref()
            .is_some_and(|record| record.status() == ApprovalStatus::Approved)
            && self.revision_from_prepared(&prepared)?.is_some();
        self.commit_view_from_prepared(
            &prepared,
            &identifier,
            status,
            &created_at,
            stored.as_ref().and_then(ApprovalRequest::actor_id),
            stored_decided_at.as_ref(),
            decided
                .as_ref()
                .and_then(|record| record.reason())
                .map(text_string)
                .as_deref(),
            committed,
        )
        .map(|mut view| {
            if view.reason.is_none() {
                view.reason = decided
                    .as_ref()
                    .and_then(|record| record.reason())
                    .map(text_string);
            }
            view
        })
    }

    /// Return every stored revision of one design in sequence order.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, the identifier is unsafe, or the
    /// limit is outside `1..=1000`.
    pub fn list_design_revisions(
        &self,
        design_id: &str,
        limit: i64,
    ) -> ApplicationResult<Vec<RevisionRecord>> {
        self.require_open()?;
        let subject = require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)?;
        let bounded = require_limit(limit, "limit", MAX_REVISION_LIMIT)?;
        let revisions = DesignRevisionRepository::new(&self.database)
            .list_for_design_with_limit(&subject, bounded)
            .map_err(repository_error)?;
        revisions
            .iter()
            .map(|revision| self.record_for(revision))
            .collect()
    }

    /// Return one stored revision together with its provenance record.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, the identifier is unsafe, the
    /// revision does not exist, or its provenance is unavailable.
    pub fn get_design_revision(&self, revision_id: &str) -> ApplicationResult<RevisionRecord> {
        self.require_open()?;
        let normalized = require_identifier(revision_id, "revision_id", MAX_IDENTIFIER_LENGTH)?;
        let revision = DesignRevisionRepository::new(&self.database)
            .get(&normalized)
            .map_err(repository_error)?
            .ok_or_else(|| {
                ApplicationError::unknown_revision(format!("revision {normalized} does not exist"))
            })?;
        self.record_for(&revision)
    }

    // -----------------------------------------------------------------
    // Mechanical reference workflow
    // -----------------------------------------------------------------

    /// Run the mechanical reference workflow up to its policy checkpoint.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, an identifier is unsafe or
    /// reserved, the workflow is already terminal, or a bounded step count is
    /// exceeded.
    pub fn run_mechanical_reference_workflow(
        &self,
        workflow_id: Option<&str>,
        design_id: &str,
        actor_id: &str,
    ) -> ApplicationResult<MechanicalRunResult> {
        self.require_open()?;
        let subject = require_identifier(design_id, "design_id", MAX_DESIGN_ID_LENGTH)?;
        let workflow = self.workflow_identifier(workflow_id, &subject)?;
        let actor = require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)?;
        let (engine, event_queue) = self.mechanical_engine(&workflow)?;
        let mut state = engine.state();
        if let Some(current) = &state {
            if current.is_terminal() {
                return Err(ApplicationError::workflow_finished(format!(
                    "workflow {workflow} already reached {}",
                    current.status().as_str()
                )));
            }
        }
        let started = state.is_none();
        let mut evaluations: Vec<EvaluationResult> = Vec::new();
        let mut checkpoint: Option<CheckpointView> = None;
        let mut policy_status: Option<PolicyDecisionStatus> = None;
        let mut policy_reason: Option<String> = None;
        self.unit_of_work(|application| {
            if state.is_none() {
                application.record_plan(&workflow)?;
                let payload = CanonicalValue::object([
                    (
                        "definition_id",
                        CanonicalValue::text(kala_domains::mechanical::WORKFLOW_DEFINITION_ID),
                    ),
                    (
                        "definition_version",
                        CanonicalValue::text(kala_domains::mechanical::WORKFLOW_VERSION),
                    ),
                    ("design_id", CanonicalValue::text(subject.as_str())),
                    ("model", CanonicalValue::text(application.model_id())),
                    ("offline", CanonicalValue::Bool(application.offline())),
                    ("provider", CanonicalValue::text(application.provider_id())),
                    (
                        "step_ids",
                        CanonicalValue::array(
                            kala_domains::mechanical::REFERENCE_WORKFLOW_STEP_IDS
                                .iter()
                                .map(|value| CanonicalValue::text(value.as_app_str())),
                        ),
                    ),
                ]);
                application.append_event(
                    "workflow.started",
                    &workflow,
                    &payload,
                    Some(&workflow),
                )?;
                state = Some(
                    engine
                        .start(&text(&workflow), None)
                        .map_err(workflow_error)?,
                );
                application.flush_workflow_events(&event_queue)?;
                application.snapshot(state.as_ref().expect("started"))?;
            }

            evaluations = application.load_evaluations(&workflow)?;
            let mut executed = 0usize;
            while let Some(current) = &state {
                if current.is_terminal() || current.pending_approval().is_some() {
                    break;
                }
                executed += 1;
                if executed > MAX_REFERENCE_STEPS {
                    return Err(ApplicationError::application(
                        "the reference workflow exceeded its bounded step count",
                    ));
                }
                let step_id = next_step_id(current)?;
                let binding = application.binding(MECHANICAL_DOMAIN)?;
                let evaluation = binding.evaluate_step(&step_id, &subject)?;
                let payload = CanonicalValue::object([
                    (
                        "evaluation",
                        evaluation
                            .to_canonical_value()
                            .map_err(|error| ApplicationError::application(error.to_string()))?,
                    ),
                    ("passed", CanonicalValue::Bool(evaluation.passed())),
                    ("step_id", CanonicalValue::text(step_id.as_str())),
                ]);
                application.append_event(
                    "workflow.step_evaluated",
                    &workflow,
                    &payload,
                    Some(&workflow),
                )?;
                evaluations.push(evaluation);
                let next = if evaluations.last().expect("pushed").passed() {
                    engine.run(state.as_ref(), None, None, None)
                } else {
                    engine.record_step_failure(
                        state.as_ref().expect("running"),
                        &text(&step_id),
                        Some(&text(HARD_CONSTRAINT_CODE)),
                        None,
                    )
                }
                .map_err(workflow_error)?;
                application.flush_workflow_events(&event_queue)?;
                state = Some(next);
                application.snapshot(state.as_ref().expect("advanced"))?;
            }

            let current = state.as_ref().expect("the workflow has a state");
            if current.pending_approval().is_some() {
                let (view, status, reason) =
                    application.register_checkpoint(current, &workflow, &actor)?;
                checkpoint = Some(view);
                policy_status = Some(status);
                policy_reason = Some(reason);
            }
            Ok(())
        })?;
        let current = state.expect("the workflow has a state");
        Ok(MechanicalRunResult {
            workflow_id: workflow.clone(),
            design_id: subject,
            status: current.status(),
            state: state_view(&current),
            evaluations,
            checkpoint,
            policy_status,
            policy_reason,
            planned_task_ids: self.planned_task_ids(&workflow)?,
            started,
            revision: None,
            budget: self.budget_view(),
        })
    }

    /// Return the current state of one stored workflow.
    ///
    /// # Errors
    ///
    /// Fails when the identifier is reserved or unsafe, or the workflow has not
    /// been started.
    pub fn get_workflow_state(&self, workflow_id: &str) -> ApplicationResult<WorkflowStateView> {
        self.require_open()?;
        let workflow = require_workflow_identifier(workflow_id)?;
        Ok(state_view(&self.load_state(&workflow)?))
    }

    /// Return every recorded policy checkpoint of one workflow.
    ///
    /// # Errors
    ///
    /// Fails when the identifier is reserved or unsafe.
    pub fn list_workflow_checkpoints(
        &self,
        workflow_id: &str,
    ) -> ApplicationResult<Vec<ApprovalRequest>> {
        self.require_open()?;
        let workflow = require_workflow_identifier(workflow_id)?;
        ApprovalRequestRepository::new(&self.database)
            .list_for_workflow(&workflow)
            .map_err(repository_error)
    }

    /// Approve the pending checkpoint and drive the workflow to completion.
    ///
    /// # Errors
    ///
    /// See [`Self::decide_workflow_checkpoint`].
    pub fn approve_workflow_checkpoint(
        &self,
        workflow_id: &str,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
        reason: Option<&str>,
    ) -> ApplicationResult<MechanicalRunResult> {
        self.decide_workflow_checkpoint(
            workflow_id,
            "approve",
            actor_id,
            state_digest,
            action_digest,
            reason,
        )
    }

    /// Reject the pending checkpoint and end the workflow as failed.
    ///
    /// # Errors
    ///
    /// See [`Self::decide_workflow_checkpoint`].
    pub fn reject_workflow_checkpoint(
        &self,
        workflow_id: &str,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
        reason: Option<&str>,
    ) -> ApplicationResult<MechanicalRunResult> {
        self.decide_workflow_checkpoint(
            workflow_id,
            "reject",
            actor_id,
            state_digest,
            action_digest,
            reason,
        )
    }

    /// Record an approve or reject decision for the pending checkpoint.
    ///
    /// # Errors
    ///
    /// Fails when the workflow is unknown, finished, or has no pending
    /// checkpoint; when a supplied digest is stale; when the policy refuses the
    /// decision; or when the authorizing commit conflicts with another writer.
    pub fn decide_workflow_checkpoint(
        &self,
        workflow_id: &str,
        decision: &str,
        actor_id: &str,
        state_digest: Option<&str>,
        action_digest: Option<&str>,
        reason: Option<&str>,
    ) -> ApplicationResult<MechanicalRunResult> {
        self.require_open()?;
        let workflow = require_workflow_identifier(workflow_id)?;
        let normalized_decision = require_identifier(decision, "decision", MAX_IDENTIFIER_LENGTH)?;
        if !CHECKPOINT_DECISIONS.contains(&normalized_decision.as_str()) {
            return Err(ApplicationError::invalid_parameters(format!(
                "decision must be one of {}",
                CHECKPOINT_DECISIONS.join(", ")
            )));
        }
        let actor = require_identifier(actor_id, "actor_id", MAX_IDENTIFIER_LENGTH)?;
        let (engine, event_queue) = self.mechanical_engine(&workflow)?;
        let mut state = engine.state().ok_or_else(|| {
            ApplicationError::unknown_workflow(format!("workflow {workflow} has not been started"))
        })?;
        if state.is_terminal() {
            return Err(ApplicationError::workflow_finished(format!(
                "workflow {workflow} already reached {}",
                state.status().as_str()
            )));
        }
        let approval = state.pending_approval().cloned().ok_or_else(|| {
            ApplicationError::checkpoint_not_pending(format!(
                "workflow {workflow} has no pending checkpoint"
            ))
        })?;
        if approval.status() != ApprovalStatus::Pending {
            return Err(ApplicationError::checkpoint_not_pending(format!(
                "workflow {workflow} has no pending checkpoint"
            )));
        }
        if let Some(state_digest) = state_digest {
            if state_digest != text_ref(approval.state_digest()) {
                return Err(ApplicationError::checkpoint_digest_mismatch(
                    "the supplied state digest is stale",
                ));
            }
        }
        if let Some(action_digest) = action_digest {
            if action_digest != text_ref(approval.action_digest()) {
                return Err(ApplicationError::checkpoint_digest_mismatch(
                    "the supplied action digest is stale",
                ));
            }
        }
        let request = policy_request(&approval, &workflow)?;
        let created_at = self.checkpoint_created_at(text_ref(request.request_id()))?;
        let decision_note = optional_text(reason, "reason", MAX_SUMMARY_LENGTH)?;
        let outcome = self.unit_of_work(|application| {
            let mut record = application
                .policy
                .get_approval(request.request_id(), None, None)
                .map_err(policy_application_error)?;
            if record.is_none() {
                record = Some(
                    application
                        .policy
                        .request_approval(&request, None)
                        .map_err(policy_application_error)?,
                );
            }
            let record = record.expect("requested");
            if record.status() != ApprovalStatus::Pending {
                return Err(ApplicationError::checkpoint_decision_denied(
                    "a checkpoint decision was already recorded for this workflow",
                ));
            }
            let precheck = application
                .policy
                .decide(&request, None)
                .map_err(policy_application_error)?;
            if is_undecidable(precheck.reason()) {
                return Err(ApplicationError::checkpoint_decision_denied(format!(
                    "the policy engine will not accept a checkpoint decision: {}",
                    precheck.reason().as_str()
                )));
            }
            let (decision_result, resolved_status, selected_option, resolved_reason) =
                if normalized_decision == "approve" {
                    let decision = application.approve_policy(
                        &request,
                        (
                            text_ref(approval.state_digest()),
                            text_ref(approval.action_digest()),
                        ),
                        &actor,
                    )?;
                    let approved = engine
                        .approve_checkpoint(
                            Some(&state),
                            Some(approval.approval_id()),
                            &CheckpointContext {
                                state_digest: Some(approval.state_digest().clone()),
                                action_digest: Some(approval.action_digest().clone()),
                                actor_id: text(&actor),
                                reason: None,
                                now: None,
                            },
                        )
                        .map_err(workflow_error)?;
                    application.flush_workflow_events(&event_queue)?;
                    let resumed = engine.resume(Some(&approved)).map_err(workflow_error)?;
                    application.flush_workflow_events(&event_queue)?;
                    state = resumed;
                    (
                        decision,
                        ApprovalStatus::Approved,
                        "approve",
                        decision_note.clone(),
                    )
                } else {
                    let decision = application.reject_policy(
                        &request,
                        (
                            text_ref(approval.state_digest()),
                            text_ref(approval.action_digest()),
                        ),
                        &actor,
                        decision_note.as_deref(),
                    )?;
                    let reject_reason = decision_note
                        .clone()
                        .unwrap_or_else(|| DEFAULT_REJECT_REASON.to_owned());
                    let rejected = engine
                        .reject_checkpoint(
                            Some(&state),
                            Some(approval.approval_id()),
                            &CheckpointContext {
                                state_digest: Some(approval.state_digest().clone()),
                                action_digest: Some(approval.action_digest().clone()),
                                actor_id: text(&actor),
                                reason: Some(text(&reject_reason)),
                                now: None,
                            },
                        )
                        .map_err(workflow_error)?;
                    application.flush_workflow_events(&event_queue)?;
                    state = rejected;
                    (
                        decision,
                        ApprovalStatus::Rejected,
                        "reject",
                        Some(reject_reason),
                    )
                };
            application.snapshot(&state)?;
            let decided_at = std::cmp::max(application.now()?, chrono_from_contract(&created_at));
            let resolved = WorkflowApproval::from_parts(
                approval.approval_id().clone(),
                approval.transition_id().clone(),
                approval.step_id().clone(),
                approval.checkpoint_type(),
                approval.state_digest().clone(),
                approval.action_digest().clone(),
                resolved_status,
                Some(text(&actor)),
                Some(kernel_from_chrono(decided_at)),
                resolved_reason.clone().map(|value| text(&value)),
            )
            .map_err(workflow_error)?;
            application.save_checkpoint_approval(
                &request,
                &approval,
                &workflow,
                resolved_status,
                selected_option,
                &actor,
                &created_at,
                &contract_timestamp(kernel_from_chrono(decided_at)),
            )?;
            application.require_stored_decision(
                text_ref(request.request_id()),
                resolved_status,
                &actor,
            )?;
            let payload = CanonicalValue::object([
                ("actor_id", CanonicalValue::text(actor.as_str())),
                (
                    "approval_id",
                    CanonicalValue::text(text_ref(approval.approval_id())),
                ),
                (
                    "policy_reason",
                    CanonicalValue::text(decision_result.reason().as_str()),
                ),
                (
                    "policy_status",
                    CanonicalValue::text(decision_result.status().as_str()),
                ),
                ("selected_option", CanonicalValue::text(selected_option)),
                (
                    "state_digest",
                    CanonicalValue::text(text_ref(approval.state_digest())),
                ),
                (
                    "action_digest",
                    CanonicalValue::text(text_ref(approval.action_digest())),
                ),
                ("status", CanonicalValue::text(state.status().as_str())),
                (
                    "step_id",
                    CanonicalValue::text(text_ref(approval.step_id())),
                ),
            ]);
            application.append_event(
                "workflow.checkpoint_decided",
                &workflow,
                &payload,
                Some(&workflow),
            )?;
            Ok((
                resolved,
                decision_result.status(),
                decision_result.reason().as_str().to_owned(),
            ))
        })?;
        let (resolved, policy_status, policy_reason) = outcome;
        let design_id = self.design_id_for(&workflow)?;
        let mut revision = None;
        if state.status() == WorkflowStatus::Completed {
            revision = Some(self.commit_workflow_revision(&design_id, &workflow, &actor)?);
        }
        Ok(MechanicalRunResult {
            workflow_id: workflow.clone(),
            design_id,
            status: state.status(),
            state: state_view(&state),
            evaluations: self.load_evaluations(&workflow)?,
            checkpoint: Some(approval_view(&resolved)),
            policy_status: Some(policy_status),
            policy_reason: Some(policy_reason),
            planned_task_ids: self.planned_task_ids(&workflow)?,
            started: false,
            revision,
            budget: self.budget_view(),
        })
    }

    // -----------------------------------------------------------------
    // Journal and lifecycle
    // -----------------------------------------------------------------

    /// Return a bounded page of journal events in sequence order.
    ///
    /// # Errors
    ///
    /// Fails when the application is closed, a filter is unsafe or reserved, a
    /// bound is outside its range, or the stored chain cannot be verified.
    pub fn list_events(
        &self,
        after_sequence: i64,
        limit: i64,
        workflow_id: Option<&str>,
        event_type: Option<&str>,
        aggregate_id: Option<&str>,
    ) -> ApplicationResult<EventPage> {
        self.require_open()?;
        if after_sequence < 0 {
            return Err(ApplicationError::invalid_parameters(
                "after_sequence must be nonnegative",
            ));
        }
        let bounded = require_limit(limit, "limit", MAX_EVENT_LIMIT)?;
        let workflow = match workflow_id {
            None => None,
            Some(value) => Some(require_workflow_identifier(value)?),
        };
        let normalized_type = match event_type {
            None => None,
            Some(value) => Some(require_identifier(
                value,
                "event_type",
                MAX_IDENTIFIER_LENGTH,
            )?),
        };
        let aggregate = match aggregate_id {
            None => None,
            Some(value) => Some(require_identifier(
                value,
                "aggregate_id",
                MAX_IDENTIFIER_LENGTH,
            )?),
        };
        let events = self
            .journal()?
            .read_filtered(&EventFilter {
                event_type: normalized_type,
                aggregate_id: aggregate,
                workflow_id: workflow,
                run_id: None,
                after_sequence,
                limit: Some(bounded),
            })
            .map_err(journal_error)?;
        let mut views = Vec::with_capacity(events.len());
        let mut truncated = false;
        for event in &events {
            let (view, omitted) = event_view(event)?;
            truncated = truncated || omitted;
            views.push(view);
        }
        let next_sequence = views.last().map_or(after_sequence, |view| view.sequence);
        Ok(EventPage {
            count: views.len(),
            events: views,
            after_sequence,
            next_after_sequence: next_sequence,
            payload_truncated: truncated,
        })
    }

    /// Release every resource this application created itself.
    ///
    /// Every operation already recorded its kernel state in its own
    /// transaction; the flush here only adds usage charged outside one. The
    /// database is released even when the flush fails.
    ///
    /// # Errors
    ///
    /// Returns the kernel-state failure while still marking the application
    /// closed and closing an owned database.
    pub fn close(&self) -> ApplicationResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let failure: RefCell<Option<ApplicationError>> = RefCell::new(None);
        let outcome = self
            .database
            .transaction(|_connection| match self.persist_kernel_state() {
                Ok(()) => Ok(()),
                Err(error) => {
                    *failure.borrow_mut() = Some(error);
                    Err(PersistenceError::InvalidJsonObject)
                }
            });
        self.closed.store(true, Ordering::SeqCst);
        let mut failure = failure.into_inner();
        if self.owns_database {
            if let Err(error) = self.database.close() {
                if failure.is_none() {
                    failure = Some(ApplicationError::application(error.to_string()));
                }
            }
        }
        match (failure, outcome) {
            (Some(error), _) => Err(error),
            (None, Ok(())) => Ok(()),
            (None, Err(error)) => Err(ApplicationError::application(error.to_string())),
        }
    }

    // -----------------------------------------------------------------
    // Private: bindings, identifiers and views
    // -----------------------------------------------------------------

    fn binding(&self, domain: &str) -> ApplicationResult<&DomainBinding> {
        self.require_open()?;
        let normalized = require_identifier(domain, "domain", MAX_IDENTIFIER_LENGTH)?;
        self.domains
            .iter()
            .find(|binding| binding.domain().as_str() == normalized)
            .ok_or_else(|| {
                ApplicationError::unknown_domain(format!(
                    "domain {normalized} is not registered; declared domains are {}",
                    self.domains().join(", ")
                ))
            })
    }

    fn sorted_bindings(&self) -> Vec<&DomainBinding> {
        let mut bindings: Vec<&DomainBinding> = self.domains.iter().collect();
        bindings.sort_by_key(|binding| binding.domain().as_str());
        bindings
    }

    fn require_open(&self) -> ApplicationResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(ApplicationError::closed_application(
                "the application is closed",
            ));
        }
        Ok(())
    }

    fn now(&self) -> ApplicationResult<DateTime<Utc>> {
        Ok((self.clock)().with_timezone(&Utc))
    }

    fn journal_handle(&self) -> ApplicationResult<EventJournal<'_>> {
        EventJournal::new(&self.journal_database).map_err(journal_error)
    }

    fn append_event(
        &self,
        event_type: &str,
        aggregate_id: &str,
        payload: &CanonicalValue,
        workflow_id: Option<&str>,
    ) -> ApplicationResult<()> {
        let mut options = AppendOptions::default();
        if let Some(workflow_id) = workflow_id {
            options = options.with_workflow_id(workflow_id.to_owned());
        }
        self.journal_handle()?
            .append_value(event_type, aggregate_id, payload, &options)
            .map(|_| ())
            .map_err(journal_error)
    }

    fn unit_of_work<T>(
        &self,
        operation: impl FnOnce(&Self) -> ApplicationResult<T>,
    ) -> ApplicationResult<T> {
        let policy_state = self.policy.export_state();
        let failure: RefCell<Option<ApplicationError>> = RefCell::new(None);
        let outcome = self
            .database
            .transaction(|_connection| match operation(self) {
                Ok(value) => match self.persist_kernel_state() {
                    Ok(()) => Ok(value),
                    Err(error) => {
                        *failure.borrow_mut() = Some(error);
                        Err(PersistenceError::InvalidJsonObject)
                    }
                },
                Err(error) => {
                    *failure.borrow_mut() = Some(error);
                    Err(PersistenceError::InvalidJsonObject)
                }
            });
        match outcome {
            Ok(value) => Ok(value),
            Err(error) => {
                if let Err(restore) = self.policy.import_state_typed(&policy_state) {
                    return Err(ApplicationError::application(restore.to_string()));
                }
                match failure.into_inner() {
                    Some(error) => Err(error),
                    None => Err(ApplicationError::application(error.to_string())),
                }
            }
        }
    }

    fn persist_kernel_state(&self) -> ApplicationResult<()> {
        let policy_state = self.policy.export_state().as_value();
        self.save_kernel_state(POLICY_STATE_AGGREGATE, &policy_state)?;
        let budget_state = budget_state_value(self.budget.export_state())?;
        self.save_kernel_state(BUDGET_STATE_AGGREGATE, &budget_state)
    }

    fn save_kernel_state(&self, aggregate: &str, state: &CanonicalValue) -> ApplicationResult<()> {
        let snapshot_id = format!(
            "{}-{}",
            kernel_state_aggregate(aggregate)?,
            random_hex_12()?
        );
        SnapshotStore::new(&self.database)
            .save_value(
                &snapshot_id,
                &kernel_state_aggregate(aggregate)?,
                state,
                Some(self.now()?),
            )
            .map(|_| ())
            .map_err(journal_error)
    }

    fn restore_kernel_state(&self) -> ApplicationResult<()> {
        let recorded_history = self
            .journal_handle()?
            .verify_chain()
            .map_err(journal_error)?
            .event_count
            > 0;
        for (aggregate, subject) in [
            (POLICY_STATE_AGGREGATE, "policy"),
            (BUDGET_STATE_AGGREGATE, "budget"),
        ] {
            let state = self.read_kernel_state(aggregate, subject)?;
            let Some(state) = state else {
                if recorded_history {
                    return Err(ApplicationError::corrupt_kernel_state(format!(
                        "the recorded {subject} state is missing from a local data directory that \
                         already holds recorded history; the launch stops rather than restoring \
                         default {subject} state"
                    )));
                }
                continue;
            };
            match subject {
                "policy" => self.policy.import_state(&state).map_err(|_| {
                    ApplicationError::corrupt_kernel_state(format!(
                        "the recorded {subject} state cannot be restored"
                    ))
                })?,
                _ => {
                    let value = serde_json::from_str::<serde_json::Value>(&canonical_dumps(
                        &state, 1_048_576,
                    )?)
                    .map_err(|_| {
                        ApplicationError::corrupt_kernel_state(format!(
                            "the recorded {subject} state cannot be restored"
                        ))
                    })?;
                    self.budget.import_state(&value).map_err(|_| {
                        ApplicationError::corrupt_kernel_state(format!(
                            "the recorded {subject} state cannot be restored"
                        ))
                    })?;
                }
            }
        }
        Ok(())
    }

    fn read_kernel_state(
        &self,
        aggregate: &str,
        subject: &str,
    ) -> ApplicationResult<Option<CanonicalValue>> {
        let state = self.read_kernel_state_record(&kernel_state_aggregate(aggregate)?, subject)?;
        if state.is_some() {
            return Ok(state);
        }
        self.read_kernel_state_record(aggregate, subject)
    }

    fn read_kernel_state_record(
        &self,
        aggregate: &str,
        subject: &str,
    ) -> ApplicationResult<Option<CanonicalValue>> {
        let snapshot = SnapshotStore::new(&self.database)
            .latest(aggregate)
            .map_err(|_| {
                ApplicationError::corrupt_kernel_state(format!(
                    "the recorded {subject} state failed verification"
                ))
            })?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let decoded = parse_json_object(&snapshot.state_json).map_err(|_| {
            ApplicationError::corrupt_kernel_state(format!(
                "the recorded {subject} state is not readable"
            ))
        })?;
        if !matches!(decoded, CanonicalValue::Object(_)) {
            return Err(ApplicationError::corrupt_kernel_state(format!(
                "the recorded {subject} state is not a JSON object"
            )));
        }
        Ok(Some(decoded))
    }

    // -----------------------------------------------------------------
    // Private: revision derivation and commit views
    // -----------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn derive_commit(
        &self,
        binding: &DomainBinding,
        subject: &str,
        summary: Option<&str>,
        parameters: Option<&CanonicalValue>,
        parent_revision_id: Option<&str>,
        parent_provenance_ids: &[String],
        transformation_ids: &[String],
        workflow_id: Option<String>,
        publish: bool,
    ) -> ApplicationResult<DerivedCommit> {
        let revisions = DesignRevisionRepository::new(&self.database);
        let head = revisions
            .latest_for_design(subject)
            .map_err(repository_error)?;
        let head_revision_id = head
            .as_ref()
            .map(|revision| revision.revision_id().to_owned());
        let parent_revision_id = match parent_revision_id {
            None => head_revision_id.clone(),
            Some(parent_revision_id) => Some(require_identifier(
                parent_revision_id,
                "parent_revision_id",
                MAX_IDENTIFIER_LENGTH,
            )?),
        };
        let parent = match &parent_revision_id {
            None => None,
            Some(parent_revision_id) => Some(
                revisions
                    .get(parent_revision_id)
                    .map_err(repository_error)?
                    .ok_or_else(|| {
                        ApplicationError::unknown_revision(format!(
                            "parent revision {parent_revision_id} does not exist"
                        ))
                    })?,
            ),
        };
        if parent_revision_id != head_revision_id {
            return Err(ApplicationError::revision_fork_refused(format!(
                "design {subject} has a head of {}; a commit must extend that head, not {}",
                head_revision_id.as_deref().unwrap_or("none"),
                parent_revision_id.as_deref().unwrap_or("none")
            )));
        }
        let mut parents = parent_provenance_ids
            .iter()
            .map(|value| require_identifier(value, "parent_provenance_id", MAX_IDENTIFIER_LENGTH))
            .collect::<ApplicationResult<Vec<_>>>()?;
        if parents.is_empty() {
            if let Some(provenance_id) = parent
                .as_ref()
                .and_then(|revision| revision.provenance_id())
            {
                parents = vec![provenance_id.to_owned()];
            }
        }
        let mut transformations: BTreeSet<String> = transformation_ids
            .iter()
            .map(|value| require_identifier(value, "transformation_id", MAX_IDENTIFIER_LENGTH))
            .collect::<ApplicationResult<BTreeSet<_>>>()?;
        transformations.insert(TRANSFORM_REFERENCE_EVALUATION.to_owned());
        let transformations: Vec<String> = transformations.into_iter().collect();

        let model = binding.parameters(parameters)?;
        let report = binding.evaluate(subject, &model)?;
        let sequence = head.as_ref().map_or_else(
            || BigInt::from(0),
            |revision| revision.sequence().as_bigint() + 1,
        );
        let revision_id = format!("rev-{subject}-r{sequence}");
        let provenance_id = format!("prov-{subject}-r{sequence}");
        let report_json = canonical_dumps(&report_payload(&report, &model)?, MAX_REPORT_BYTES)?;
        let encoded = format!("{report_json}\n");
        let digest = if publish {
            let digest = self
                .artifact_store
                .put_bytes(encoded.as_bytes())
                .map_err(|error| ApplicationError::application(error.to_string()))?;
            self.budget
                .record_artifact_bytes(encoded.len())
                .map_err(|error| ApplicationError::application(error.to_string()))?;
            digest
        } else {
            let digest = sha256_hex_text(&encoded);
            if !self
                .artifact_store
                .contains(&digest)
                .map_err(|error| ApplicationError::application(error.to_string()))?
            {
                return Err(ApplicationError::application(format!(
                    "the report artifact a revision commit was prepared against is not in the \
                     store: {digest}"
                )));
            }
            digest
        };
        let artifact = ArtifactRef::from_value(&CanonicalValue::object([
            (
                "artifact_id",
                CanonicalValue::text(format!("artifact-{}-report", binding.domain().as_str())),
            ),
            (
                "uri",
                CanonicalValue::text(format!("{ARTIFACT_SCHEME_PREFIX}{digest}")),
            ),
            ("digest", CanonicalValue::text(digest.as_str())),
            ("media_type", CanonicalValue::text(REPORT_MEDIA_TYPE)),
            (
                "size_bytes",
                CanonicalValue::integer(BigInt::from(encoded.len())),
            ),
            (
                "classification",
                CanonicalValue::text(REPORT_CLASSIFICATION.as_str()),
            ),
            (
                "provenance_id",
                CanonicalValue::text(provenance_id.as_str()),
            ),
        ]))
        .map_err(|error| ApplicationError::application(error.to_string()))?;
        let spec = design_spec(binding, subject, &report, &model, summary)?;
        let package_digest = binding.package().digest().to_owned();
        let spec_digest = canonical_digest(
            &spec
                .to_canonical_value()
                .map_err(|error| ApplicationError::application(error.to_string()))?,
        )?;
        let provenance_inputs = Provenance::from_value(&CanonicalValue::object([
            (
                "provenance_id",
                CanonicalValue::text(provenance_id.as_str()),
            ),
            (
                "subject_ids",
                CanonicalValue::array([CanonicalValue::text(subject)]),
            ),
            (
                "output_artifact_ids",
                CanonicalValue::array([CanonicalValue::text(artifact.artifact_id())]),
            ),
            (
                "domain_package_digests",
                CanonicalValue::array([CanonicalValue::text(package_digest.as_str())]),
            ),
            (
                "parent_provenance_ids",
                CanonicalValue::array(
                    parents
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "transformation_ids",
                CanonicalValue::array(
                    transformations
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "recorded_at",
                CanonicalValue::text("1970-01-01T00:00:00+00:00"),
            ),
        ]))
        .map_err(|error| ApplicationError::application(error.to_string()))?;

        let head_payload = match &head {
            None => CanonicalValue::Null,
            Some(head) => CanonicalValue::object([
                ("revision_id", CanonicalValue::text(head.revision_id())),
                (
                    "sequence",
                    CanonicalValue::integer(head.sequence().as_bigint().clone()),
                ),
                (
                    "spec_digest",
                    CanonicalValue::text(canonical_digest(
                        &head
                            .spec()
                            .to_canonical_value()
                            .map_err(|error| ApplicationError::application(error.to_string()))?,
                    )?),
                ),
            ]),
        };
        let state_payload = CanonicalValue::object([
            (
                "artifact_digests",
                CanonicalValue::array([CanonicalValue::text(artifact.digest().to_hex())]),
            ),
            (
                "artifact_metadata",
                CanonicalValue::array([CanonicalValue::object([
                    ("artifact_id", CanonicalValue::text(artifact.artifact_id())),
                    (
                        "classification",
                        CanonicalValue::text(artifact.classification().as_str()),
                    ),
                    ("digest", CanonicalValue::text(artifact.digest().to_hex())),
                    ("media_type", CanonicalValue::text(artifact.media_type())),
                    (
                        "size_bytes",
                        CanonicalValue::integer(artifact.size_bytes().as_bigint().clone()),
                    ),
                ])]),
            ),
            ("design_head", head_payload),
            ("design_id", CanonicalValue::text(subject)),
            ("domain", CanonicalValue::text(binding.domain().as_str())),
            (
                "package_digests",
                CanonicalValue::array([CanonicalValue::text(package_digest.as_str())]),
            ),
            (
                "parameters_digest",
                CanonicalValue::text(report.parameters_digest.as_str()),
            ),
            (
                "parent_provenance_ids",
                CanonicalValue::array(
                    parents
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "parent_revision_id",
                parent_revision_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::text(value.as_str())
                    }),
            ),
            (
                "provenance_id",
                CanonicalValue::text(provenance_id.as_str()),
            ),
            ("spec_digest", CanonicalValue::text(spec_digest.as_str())),
            (
                "transformation_ids",
                CanonicalValue::array(
                    transformations
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
        ]);
        let state_digest = canonical_digest(&state_payload)?;
        let action_payload = CanonicalValue::object([
            (
                "capabilities",
                CanonicalValue::array(
                    PRE_COMMIT_CAPABILITIES
                        .iter()
                        .map(|value| CanonicalValue::text(*value)),
                ),
            ),
            (
                "checkpoint_type",
                CanonicalValue::text(REVISION_COMMIT_CHECKPOINT.as_str()),
            ),
            ("design_id", CanonicalValue::text(subject)),
            ("domain", CanonicalValue::text(binding.domain().as_str())),
            (
                "expected_head_revision_id",
                head_revision_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::text(value.as_str())
                    }),
            ),
            ("operation", CanonicalValue::text(REVISION_COMMIT_OPERATION)),
            (
                "planned_revision_id",
                CanonicalValue::text(revision_id.as_str()),
            ),
            (
                "planned_sequence",
                CanonicalValue::integer(sequence.clone()),
            ),
            ("spec_digest", CanonicalValue::text(spec_digest.as_str())),
            ("state_digest", CanonicalValue::text(state_digest.as_str())),
            (
                "workflow_id",
                workflow_id.as_ref().map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::text(value.as_str())
                }),
            ),
        ]);
        let action_digest = canonical_digest(&action_payload)?;
        let request_id = format!(
            "{REVISION_COMMIT_PREFIX}{subject}-{}-{}",
            &state_digest[..DIGEST_PREFIX_LENGTH],
            &action_digest[..DIGEST_PREFIX_LENGTH]
        );
        let parameters_value = crate::domains::parameters_value(&model)?;
        Ok(DerivedCommit {
            design_id: subject.to_owned(),
            domain: binding.domain().as_str().to_owned(),
            revision_id,
            provenance_id,
            sequence,
            parent_revision_id,
            expected_head_revision_id: head_revision_id,
            spec,
            artifact,
            domain_package: binding.package().manifest().clone(),
            provenance_inputs,
            parameters: parameters_value,
            spec_digest,
            parameters_digest: report.parameters_digest,
            package_digests: vec![package_digest],
            parent_provenance_ids: parents,
            transformation_ids: transformations,
            workflow_id,
            state_digest,
            action_digest,
            request_id,
        })
    }

    fn prepared_payload(
        &self,
        derived: &DerivedCommit,
        created_at: Timestamp,
    ) -> ApplicationResult<CanonicalValue> {
        Ok(CanonicalValue::object([
            (
                "action_digest",
                CanonicalValue::text(derived.action_digest.as_str()),
            ),
            (
                "artifact_digests",
                CanonicalValue::array([CanonicalValue::text(derived.artifact.digest().to_hex())]),
            ),
            (
                "created_at",
                CanonicalValue::text(
                    kernel_from_chrono(chrono_from_contract(&created_at)).isoformat(),
                ),
            ),
            (
                "design_id",
                CanonicalValue::text(derived.design_id.as_str()),
            ),
            ("domain", CanonicalValue::text(derived.domain.as_str())),
            (
                "expected_head_revision_id",
                derived
                    .expected_head_revision_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::text(value.as_str())
                    }),
            ),
            (
                "package_digests",
                CanonicalValue::array(
                    derived
                        .package_digests
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            ("parameters", derived.parameters.clone()),
            (
                "parameters_digest",
                CanonicalValue::text(derived.parameters_digest.as_str()),
            ),
            (
                "parent_provenance_ids",
                CanonicalValue::array(
                    derived
                        .parent_provenance_ids
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "parent_revision_id",
                derived
                    .parent_revision_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::text(value.as_str())
                    }),
            ),
            (
                "request_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            (
                "revision_id",
                CanonicalValue::text(derived.revision_id.as_str()),
            ),
            (
                "sequence",
                CanonicalValue::integer(derived.sequence.clone()),
            ),
            (
                "spec",
                derived
                    .spec
                    .to_canonical_value()
                    .map_err(|error| ApplicationError::application(error.to_string()))?,
            ),
            (
                "spec_digest",
                CanonicalValue::text(derived.spec_digest.as_str()),
            ),
            (
                "state_digest",
                CanonicalValue::text(derived.state_digest.as_str()),
            ),
            (
                "summary",
                CanonicalValue::text(text_string(derived.spec.summary())),
            ),
            (
                "transformation_ids",
                CanonicalValue::array(
                    derived
                        .transformation_ids
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "workflow_id",
                derived
                    .workflow_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::text(value.as_str())
                    }),
            ),
        ]))
    }

    fn prepared_commit(&self, request_id: &str) -> ApplicationResult<CanonicalValue> {
        let events = self
            .journal_handle()?
            .read_filtered(&EventFilter {
                event_type: Some(PREPARED_REVISION_EVENT.to_owned()),
                aggregate_id: Some(request_id.to_owned()),
                workflow_id: None,
                run_id: None,
                after_sequence: 0,
                limit: Some(1),
            })
            .map_err(journal_error)?;
        events
            .first()
            .map(|event| event.payload().map_err(journal_error))
            .transpose()?
            .ok_or_else(|| {
                ApplicationError::unknown_prepared_revision(format!(
                    "revision commit {request_id} has no recorded preparation"
                ))
            })
    }

    fn prepared_created_at(&self, request_id: &str) -> ApplicationResult<Timestamp> {
        let prepared = self.prepared_commit(request_id)?;
        let stored = ApprovalRequestRepository::new(&self.database)
            .get(request_id)
            .map_err(repository_error)?;
        if let Some(stored) = stored {
            return Ok(stored.created_at().clone());
        }
        let recorded = field(&prepared, "created_at").cloned().ok_or_else(|| {
            ApplicationError::unknown_prepared_revision(format!(
                "revision commit {request_id} has no recorded creation time"
            ))
        })?;
        match &recorded {
            CanonicalValue::Text(recorded) => {
                parse_timestamp(&json_text_utf8(recorded), "created_at")
            }
            _ => Err(ApplicationError::application(
                "the recorded created_at is not a timestamp",
            )),
        }
    }

    fn rederive_commit(&self, prepared: &CanonicalValue) -> ApplicationResult<DerivedCommit> {
        let design_id = require_stored_identifier(
            field(prepared, "design_id"),
            "design_id",
            MAX_DESIGN_ID_LENGTH,
        )?;
        let domain =
            require_stored_identifier(field(prepared, "domain"), "domain", MAX_IDENTIFIER_LENGTH)?;
        let binding = self.binding(&domain)?;
        let parameters = field(prepared, "parameters")
            .filter(|value| matches!(value, CanonicalValue::Object(_)));
        let parent_revision_id = field(prepared, "parent_revision_id")
            .filter(|value| !matches!(value, CanonicalValue::Null))
            .map(python_text);
        let parent_revision_id = match parent_revision_id {
            Some(value) => Some(value?),
            None => None,
        };
        let workflow_id = field(prepared, "workflow_id")
            .filter(|value| !matches!(value, CanonicalValue::Null))
            .map(python_text);
        let workflow_id = match workflow_id {
            Some(value) => Some(value?),
            None => None,
        };
        let derived = self
            .derive_commit(
                binding,
                &design_id,
                optional_text_value(field(prepared, "summary"), "summary", MAX_SUMMARY_LENGTH)?
                    .as_deref(),
                parameters,
                parent_revision_id.as_deref(),
                &identifier_sequence(
                    field(prepared, "parent_provenance_ids"),
                    "parent_provenance_id",
                )?,
                &identifier_sequence(field(prepared, "transformation_ids"), "transformation_id")?,
                workflow_id,
                false,
            )
            .map_err(|error| {
                if error.kind() == ErrorKind::RevisionForkRefused {
                    ApplicationError::checkpoint_digest_mismatch(
                        "the design head moved after the revision commit was prepared",
                    )
                } else {
                    error
                }
            })?;
        let recorded_request = require_stored_identifier(
            field(prepared, "request_id"),
            "request_id",
            MAX_IDENTIFIER_LENGTH,
        )?;
        if derived.request_id != recorded_request {
            return Err(ApplicationError::checkpoint_digest_mismatch(format!(
                "the state prepared for revision commit {recorded_request} has changed"
            )));
        }
        Ok(derived)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_view_from_prepared(
        &self,
        prepared: &CanonicalValue,
        request_id: &str,
        status: ApprovalStatus,
        created_at: &Timestamp,
        actor_id: Option<&str>,
        decided_at: Option<&Timestamp>,
        reason: Option<&str>,
        committed: bool,
    ) -> ApplicationResult<RevisionCommitView> {
        let sequence = require_stored_sequence(field(prepared, "sequence"))?;
        let parent = field(prepared, "parent_revision_id");
        let workflow = field(prepared, "workflow_id");
        let package_digests =
            identifier_sequence(field(prepared, "package_digests"), "package_digest")?
                .iter()
                .map(|value| require_digest(value, "package digest"))
                .collect::<ApplicationResult<Vec<_>>>()?;
        let artifact_digests =
            identifier_sequence(field(prepared, "artifact_digests"), "artifact_digest")?
                .iter()
                .map(|value| require_digest(value, "artifact digest"))
                .collect::<ApplicationResult<Vec<_>>>()?;
        Ok(RevisionCommitView {
            request_id: request_id.to_owned(),
            design_id: require_stored_identifier(
                field(prepared, "design_id"),
                "design_id",
                MAX_DESIGN_ID_LENGTH,
            )?,
            domain: require_stored_identifier(
                field(prepared, "domain"),
                "domain",
                MAX_IDENTIFIER_LENGTH,
            )?,
            revision_id: require_stored_identifier(
                field(prepared, "revision_id"),
                "revision_id",
                MAX_IDENTIFIER_LENGTH,
            )?,
            sequence,
            parent_revision_id: python_optional_text(parent)?,
            state_digest: require_stored_digest(field(prepared, "state_digest"), "state digest")?,
            action_digest: require_stored_digest(
                field(prepared, "action_digest"),
                "action digest",
            )?,
            spec_digest: require_stored_digest(field(prepared, "spec_digest"), "spec digest")?,
            parameters_digest: require_stored_digest(
                field(prepared, "parameters_digest"),
                "parameters digest",
            )?,
            package_digests,
            artifact_digests,
            workflow_id: python_optional_text(workflow)?,
            status,
            actor_id: actor_id.map(str::to_owned),
            decided_at: decided_at.cloned(),
            reason: reason.map(str::to_owned),
            created_at: created_at.clone(),
            committed,
        })
    }

    fn require_undecided_commit(&self, request_id: &str) -> ApplicationResult<()> {
        let policy_record = self
            .policy
            .get_approval(&text(request_id), None, None)
            .map_err(policy_application_error)?;
        let stored_record = ApprovalRequestRepository::new(&self.database)
            .get(request_id)
            .map_err(repository_error)?;
        for record in [
            policy_record.map(|record| (record.status(), ())),
            stored_record.map(|record| (record.status(), ())),
        ]
        .into_iter()
        .flatten()
        {
            if record.0 != ApprovalStatus::Pending {
                return Err(ApplicationError::checkpoint_not_pending(format!(
                    "revision commit {request_id} is already {}",
                    record.0.as_str()
                )));
            }
        }
        Ok(())
    }

    fn workflow_commit_actor(
        &self,
        workflow_id: &str,
        derived: &DerivedCommit,
    ) -> ApplicationResult<String> {
        if derived.workflow_id.as_deref() != Some(workflow_id) {
            return Err(ApplicationError::invalid_parameters(format!(
                "revision commit {} was not prepared for workflow {workflow_id}",
                derived.request_id
            )));
        }
        let state = self.load_state(workflow_id)?;
        if state.status() != WorkflowStatus::Completed {
            return Err(ApplicationError::checkpoint_decision_denied(format!(
                "workflow {workflow_id} has not completed, so it authorizes no revision commit"
            )));
        }
        let approved: Vec<ApprovalRequest> = ApprovalRequestRepository::new(&self.database)
            .list_for_workflow(workflow_id)
            .map_err(repository_error)?
            .into_iter()
            .filter(|record| {
                record.checkpoint_type() == ApprovalKind::PreCommit
                    && record.status() == ApprovalStatus::Approved
                    && record.actor_id().is_some()
            })
            .collect();
        if approved.len() != 1 {
            return Err(ApplicationError::checkpoint_decision_denied(format!(
                "workflow {workflow_id} has no single approved pre-commit checkpoint"
            )));
        }
        Ok(approved[0].actor_id().unwrap_or(LOCAL_ACTOR_ID).to_owned())
    }

    fn commit_workflow_revision(
        &self,
        design_id: &str,
        workflow_id: &str,
        actor: &str,
    ) -> ApplicationResult<RevisionRecord> {
        let prepared = self.prepare_design_revision(
            design_id,
            MECHANICAL_DOMAIN,
            Some(
                "Committed mechanical reference evaluation after the declared pre-commit \
                 checkpoint was approved.",
            ),
            None,
            None,
            &[],
            &[TRANSFORM_MECHANICAL_RUN.to_owned()],
            Some(workflow_id),
        )?;
        self.commit_design_revision(
            &prepared.request_id,
            actor,
            Some(&prepared.state_digest),
            Some(&prepared.action_digest),
            None,
            Some(workflow_id),
        )
    }

    fn write_revision(
        &self,
        derived: &DerivedCommit,
        actor: &str,
        reason: Option<&str>,
    ) -> ApplicationResult<bool> {
        let created_at = contract_timestamp(kernel_from_chrono(self.now()?));
        let revision = derived_revision(derived, &created_at)?;
        let provenance = derived_provenance(derived, &created_at)?;
        match DesignRevisionRepository::new(&self.database)
            .commit_revision(&revision, derived.expected_head_revision_id.as_deref())
        {
            Ok(_) => {}
            Err(error) => {
                if let Some(head_moved) = error.as_head_moved() {
                    let payload = CanonicalValue::object([
                        ("actor_id", CanonicalValue::text(actor)),
                        (
                            "design_id",
                            CanonicalValue::text(derived.design_id.as_str()),
                        ),
                        (
                            "expected_head_revision_id",
                            derived
                                .expected_head_revision_id
                                .as_ref()
                                .map_or(CanonicalValue::Null, |value| {
                                    CanonicalValue::text(value.as_str())
                                }),
                        ),
                        (
                            "actual_head_revision_id",
                            head_moved
                                .actual_head_revision_id()
                                .map_or(CanonicalValue::Null, CanonicalValue::text),
                        ),
                        (
                            "planned_sequence",
                            CanonicalValue::integer(derived.sequence.clone()),
                        ),
                        (
                            "planned_revision_id",
                            CanonicalValue::text(derived.revision_id.as_str()),
                        ),
                    ]);
                    self.append_event(
                        CONFLICTED_REVISION_EVENT,
                        &derived.request_id,
                        &payload,
                        derived.workflow_id.as_deref(),
                    )?;
                    return Ok(false);
                }
                return Err(repository_error(error));
            }
        }
        let payload = CanonicalValue::object([
            (
                "action_digest",
                CanonicalValue::text(derived.action_digest.as_str()),
            ),
            ("actor_id", CanonicalValue::text(actor)),
            (
                "artifact_digests",
                CanonicalValue::array([CanonicalValue::text(derived.artifact.digest().to_hex())]),
            ),
            (
                "design_id",
                CanonicalValue::text(derived.design_id.as_str()),
            ),
            ("domain", CanonicalValue::text(derived.domain.as_str())),
            (
                "package_digest",
                CanonicalValue::text(&derived.package_digests[0]),
            ),
            (
                "parameters_digest",
                CanonicalValue::text(derived.parameters_digest.as_str()),
            ),
            (
                "provenance",
                provenance
                    .to_canonical_value()
                    .map_err(|error| ApplicationError::application(error.to_string()))?,
            ),
            (
                "request_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            (
                "revision_id",
                CanonicalValue::text(derived.revision_id.as_str()),
            ),
            (
                "sequence",
                CanonicalValue::integer(derived.sequence.clone()),
            ),
            (
                "spec_digest",
                CanonicalValue::text(derived.spec_digest.as_str()),
            ),
            (
                "state_digest",
                CanonicalValue::text(derived.state_digest.as_str()),
            ),
            (
                "reason",
                reason.map_or(CanonicalValue::Null, CanonicalValue::text),
            ),
        ]);
        self.append_event(
            COMMITTED_REVISION_EVENT,
            &derived.revision_id,
            &payload,
            derived.workflow_id.as_deref(),
        )?;
        Ok(true)
    }

    fn record_for(&self, revision: &DesignRevision) -> ApplicationResult<RevisionRecord> {
        let provenance_id = revision.provenance_id();
        let mut provenance = None;
        if provenance_id.is_some() {
            let events = self
                .journal_handle()?
                .read_filtered(&EventFilter {
                    event_type: Some(COMMITTED_REVISION_EVENT.to_owned()),
                    aggregate_id: Some(revision.revision_id().to_owned()),
                    workflow_id: None,
                    run_id: None,
                    after_sequence: 0,
                    limit: Some(1),
                })
                .map_err(journal_error)?;
            for event in events {
                let payload = event.payload().map_err(journal_error)?;
                if let Some(record) = field(&payload, "provenance") {
                    provenance = Some(Provenance::from_value(record).map_err(|_| {
                        ApplicationError::provenance_unavailable(
                            "the stored provenance record is unreadable",
                        )
                    })?);
                }
            }
        }
        let provenance = provenance.ok_or_else(|| {
            ApplicationError::provenance_unavailable(format!(
                "revision {} has no recorded provenance",
                revision.revision_id()
            ))
        })?;
        Ok(RevisionRecord {
            revision: revision.clone(),
            provenance,
            artifact_digests: revision
                .artifacts()
                .iter()
                .map(|artifact| artifact.digest().to_hex())
                .collect(),
        })
    }

    // -----------------------------------------------------------------
    // Private: workflow plumbing
    // -----------------------------------------------------------------

    fn mechanical_engine(
        &self,
        workflow_id: &str,
    ) -> ApplicationResult<(WorkflowEngine, WorkflowEventQueue)> {
        let queue: WorkflowEventQueue = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_queue = Arc::clone(&queue);
        let engine = WorkflowEngine::new(kala_domains::mechanical::MECHANICAL_WORKFLOW.clone())
            .with_journal_callback(move |event_type, workflow_id, payload| {
                let mut pending = sink_queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                pending.push((
                    text_string(event_type),
                    text_string(workflow_id),
                    payload.clone(),
                ));
                Ok::<(), kala_kernel::workflow::WorkflowError>(())
            });
        let latest = SnapshotStore::new(&self.database)
            .latest(&require_workflow_identifier(workflow_id)?)
            .map_err(journal_error)?;
        if let Some(latest) = latest {
            engine
                .restore_json(&latest.state_json)
                .map_err(workflow_error)?;
        }
        Ok((engine, queue))
    }

    fn load_state(&self, workflow_id: &str) -> ApplicationResult<WorkflowState> {
        let (engine, _queue) = self.mechanical_engine(workflow_id)?;
        engine.state().ok_or_else(|| {
            ApplicationError::unknown_workflow(format!(
                "workflow {workflow_id} has not been started"
            ))
        })
    }

    fn snapshot(&self, state: &WorkflowState) -> ApplicationResult<WorkflowSnapshot> {
        let subject = require_workflow_identifier(text_ref(state.workflow_id()))?;
        let snapshots = SnapshotStore::new(&self.database);
        let latest = snapshots.latest(&subject).map_err(journal_error)?;
        let state_json = state.to_json().map_err(workflow_error)?;
        if let Some(latest) = &latest {
            if latest.state_json == state_json {
                return Ok(latest.clone());
            }
        }
        let next_sequence = latest.as_ref().map_or(1, |snapshot| snapshot.sequence + 1);
        snapshots
            .save_value(
                &format!("snapshot-{subject}-{next_sequence:06}"),
                &subject,
                &state.to_dict(),
                Some(self.now()?),
            )
            .map_err(journal_error)
    }

    fn flush_workflow_events(&self, queue: &WorkflowEventQueue) -> ApplicationResult<()> {
        let pending: Vec<(String, String, CanonicalValue)> = {
            let mut guard = queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        for (event_type, workflow_id, payload) in pending {
            self.append_event(&event_type, &workflow_id, &payload, Some(&workflow_id))?;
        }
        Ok(())
    }

    fn approve_policy(
        &self,
        request: &PolicyRequest,
        digests: (&str, &str),
        actor: &str,
    ) -> ApplicationResult<PolicyDecision> {
        let context = DecisionContext::new()
            .with_state_digest(text(digests.0))
            .with_action_digest(text(digests.1))
            .with_actor_id(text(actor));
        let decision = self.policy.approve(request, &context).map_err(|error| {
            ApplicationError::checkpoint_decision_denied(format!(
                "the policy engine refused the checkpoint approval: {error}"
            ))
        })?;
        if !decision.allowed() {
            return Err(ApplicationError::checkpoint_decision_denied(format!(
                "the policy engine refused the checkpoint approval: {}",
                decision.reason().as_str()
            )));
        }
        Ok(decision)
    }

    fn reject_policy(
        &self,
        request: &PolicyRequest,
        digests: (&str, &str),
        actor: &str,
        reason: Option<&str>,
    ) -> ApplicationResult<PolicyDecision> {
        let mut context = DecisionContext::new()
            .with_state_digest(text(digests.0))
            .with_action_digest(text(digests.1))
            .with_actor_id(text(actor));
        if let Some(reason) = reason {
            context = context.with_reason(text(reason));
        }
        self.policy.reject(request, &context).map_err(|error| {
            ApplicationError::checkpoint_decision_denied(format!(
                "the policy engine refused the checkpoint rejection: {error}"
            ))
        })
    }

    fn register_checkpoint(
        &self,
        state: &WorkflowState,
        workflow_id: &str,
        actor: &str,
    ) -> ApplicationResult<(CheckpointView, PolicyDecisionStatus, String)> {
        let approval = state.pending_approval().cloned().ok_or_else(|| {
            ApplicationError::checkpoint_not_pending(format!(
                "workflow {workflow_id} has no pending checkpoint"
            ))
        })?;
        let request = policy_request(&approval, workflow_id)?;
        let record = self
            .policy
            .request_approval(&request, None)
            .map_err(policy_application_error)?;
        let decision = self
            .policy
            .decide(&request, None)
            .map_err(policy_application_error)?;
        if record.status() != ApprovalStatus::Pending {
            return Err(ApplicationError::checkpoint_not_pending(format!(
                "checkpoint {} is already {}",
                approval.approval_id(),
                record.status().as_str()
            )));
        }
        let awaiting = PolicyDecisionStatus::Pending;
        let reason = if decision.status() != awaiting {
            PolicyReason::ApprovalRequired
        } else {
            decision.reason()
        };
        let created_at = contract_timestamp(kernel_from_chrono(self.now()?));
        self.save_checkpoint_approval(
            &request,
            &approval,
            workflow_id,
            ApprovalStatus::Pending,
            "pending",
            "",
            &created_at,
            &created_at,
        )?;
        let payload = CanonicalValue::object([
            ("actor_id", CanonicalValue::text(actor)),
            (
                "approval_id",
                CanonicalValue::text(text_ref(approval.approval_id())),
            ),
            (
                "capabilities",
                CanonicalValue::array(
                    request
                        .requested_capabilities()
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            ("policy_status", CanonicalValue::text(awaiting.as_str())),
            ("policy_reason", CanonicalValue::text(reason.as_str())),
            ("requires_approval", CanonicalValue::Bool(true)),
            (
                "state_digest",
                CanonicalValue::text(text_ref(approval.state_digest())),
            ),
            (
                "action_digest",
                CanonicalValue::text(text_ref(approval.action_digest())),
            ),
            (
                "step_id",
                CanonicalValue::text(text_ref(approval.step_id())),
            ),
            (
                "transition_id",
                CanonicalValue::text(text_ref(approval.transition_id())),
            ),
        ]);
        self.append_event(
            "policy.checkpoint_requested",
            workflow_id,
            &payload,
            Some(workflow_id),
        )?;
        Ok((
            approval_view(&approval),
            awaiting,
            reason.as_str().to_owned(),
        ))
    }

    fn checkpoint_created_at(&self, request_id: &str) -> ApplicationResult<Timestamp> {
        let existing = ApprovalRequestRepository::new(&self.database)
            .get(request_id)
            .map_err(repository_error)?;
        match existing {
            Some(record) => Ok(record.created_at().clone()),
            None => Ok(contract_timestamp(kernel_from_chrono(self.now()?))),
        }
    }

    fn design_id_for(&self, workflow_id: &str) -> ApplicationResult<String> {
        let events = self
            .journal_handle()?
            .read_filtered(&EventFilter {
                event_type: Some("workflow.started".to_owned()),
                aggregate_id: Some(workflow_id.to_owned()),
                workflow_id: None,
                run_id: None,
                after_sequence: 0,
                limit: Some(1),
            })
            .map_err(journal_error)?;
        for event in events {
            let payload = event.payload().map_err(journal_error)?;
            if let Some(CanonicalValue::Text(design_id)) = field(&payload, "design_id") {
                return Ok(json_text_utf8(design_id));
            }
        }
        Err(ApplicationError::unknown_workflow(format!(
            "workflow {workflow_id} has no recorded design"
        )))
    }

    fn record_plan(&self, workflow_id: &str) -> ApplicationResult<()> {
        let tasks = futures::executor::block_on(self.planner.plan(MECHANICAL_PLAN_OBJECTIVE))
            .map_err(|error| {
                ApplicationError::application(format!("the planner refused the objective: {error}"))
            })?;
        let mut tool_ids: BTreeSet<String> = BTreeSet::new();
        for task in &tasks {
            tool_ids.insert(task.tool_id().to_owned());
        }
        let payload = CanonicalValue::object([
            ("objective", CanonicalValue::text(MECHANICAL_PLAN_OBJECTIVE)),
            ("planner", CanonicalValue::text("OfflinePlanner")),
            (
                "step_count",
                CanonicalValue::integer(BigInt::from(tasks.len())),
            ),
            (
                "task_ids",
                CanonicalValue::array(
                    tasks
                        .iter()
                        .map(|task| CanonicalValue::text(task.step_id())),
                ),
            ),
            (
                "tool_ids",
                CanonicalValue::array(
                    tool_ids
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
        ]);
        self.append_event("plan.created", workflow_id, &payload, Some(workflow_id))
    }

    fn planned_task_ids(&self, workflow_id: &str) -> ApplicationResult<Vec<String>> {
        let events = self
            .journal_handle()?
            .read_filtered(&EventFilter {
                event_type: Some("plan.created".to_owned()),
                aggregate_id: Some(workflow_id.to_owned()),
                workflow_id: Some(workflow_id.to_owned()),
                run_id: None,
                after_sequence: 0,
                limit: Some(1),
            })
            .map_err(journal_error)?;
        for event in events {
            let payload = event.payload().map_err(journal_error)?;
            if let Some(CanonicalValue::Array(planned)) = field(&payload, "task_ids") {
                return planned
                    .iter()
                    .map(|value| match value {
                        CanonicalValue::Text(task_id) => require_identifier(
                            &json_text_utf8(task_id),
                            "task_id",
                            MAX_IDENTIFIER_LENGTH,
                        ),
                        _ => Err(ApplicationError::application(
                            "the recorded task_id is not an identifier",
                        )),
                    })
                    .collect();
            }
        }
        Ok(Vec::new())
    }

    fn load_evaluations(&self, workflow_id: &str) -> ApplicationResult<Vec<EvaluationResult>> {
        let events = self
            .journal_handle()?
            .read_filtered(&EventFilter {
                event_type: Some("workflow.step_evaluated".to_owned()),
                aggregate_id: None,
                workflow_id: Some(workflow_id.to_owned()),
                run_id: None,
                after_sequence: 0,
                limit: Some(MAX_EVENT_LIMIT),
            })
            .map_err(journal_error)?;
        let mut evaluations = Vec::new();
        for event in events {
            let payload = event.payload().map_err(journal_error)?;
            let Some(record) = field(&payload, "evaluation") else {
                continue;
            };
            if !matches!(record, CanonicalValue::Object(_)) {
                continue;
            }
            evaluations.push(EvaluationResult::from_value(record).map_err(|_| {
                ApplicationError::application("a journalled evaluation is unreadable")
            })?);
        }
        Ok(evaluations)
    }

    fn workflow_identifier(
        &self,
        workflow_id: Option<&str>,
        design_id: &str,
    ) -> ApplicationResult<String> {
        let candidate = match workflow_id {
            None => format!("{WORKFLOW_ID_PREFIX}{design_id}"),
            Some(value) => value.to_owned(),
        };
        require_workflow_identifier(&candidate)
    }

    fn budget_view(&self) -> BudgetUsageView {
        let usage = self.budget.usage();
        BudgetUsageView {
            model_calls: counter_bigint(usage.model_calls()),
            tool_calls: counter_bigint(usage.tool_calls()),
            total_tokens: counter_bigint(usage.total_tokens()),
            artifact_bytes: counter_bigint(usage.artifact_bytes()),
            duration_seconds: usage.duration_seconds(),
            cost: usage.cost().to_python_string(),
            replanning_attempts: counter_bigint(usage.replanning_attempts()),
        }
    }

    // -----------------------------------------------------------------
    // Private: approvals
    // -----------------------------------------------------------------

    fn save_commit_approval(
        &self,
        derived: &DerivedCommit,
        request: &PolicyRequest,
        created_at: DateTime<Utc>,
    ) -> ApplicationResult<()> {
        let record = ApprovalRequest::from_value(&CanonicalValue::object([
            (
                "request_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            (
                "checkpoint_type",
                CanonicalValue::text(REVISION_COMMIT_CHECKPOINT.as_str()),
            ),
            ("status", CanonicalValue::text("pending")),
            (
                "workflow_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            ("step_id", CanonicalValue::Null),
            (
                "action_digest",
                CanonicalValue::text(derived.action_digest.as_str()),
            ),
            (
                "state_digest",
                CanonicalValue::text(derived.state_digest.as_str()),
            ),
            (
                "options",
                CanonicalValue::array(
                    APPROVAL_OPTIONS
                        .iter()
                        .map(|value| CanonicalValue::text(*value)),
                ),
            ),
            ("selected_option_id", CanonicalValue::Null),
            (
                "requested_capabilities",
                CanonicalValue::array(
                    request
                        .requested_capabilities()
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "created_at",
                CanonicalValue::text(
                    contract_timestamp(kernel_from_chrono(created_at)).to_wire_string(),
                ),
            ),
        ]))
        .map_err(|error| ApplicationError::application(error.to_string()))?;
        ApprovalRequestRepository::new(&self.database)
            .save(&record)
            .map_err(repository_error)
    }

    #[allow(clippy::too_many_arguments)]
    fn save_decided_commit_approval(
        &self,
        derived: &DerivedCommit,
        request: &PolicyRequest,
        status: ApprovalStatus,
        selected_option: &str,
        actor: &str,
        created_at: &Timestamp,
        decided_at: &Timestamp,
    ) -> ApplicationResult<()> {
        let record = ApprovalRequest::from_value(&CanonicalValue::object([
            (
                "request_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            (
                "checkpoint_type",
                CanonicalValue::text(REVISION_COMMIT_CHECKPOINT.as_str()),
            ),
            ("status", CanonicalValue::text(status.as_str())),
            (
                "workflow_id",
                CanonicalValue::text(derived.request_id.as_str()),
            ),
            ("step_id", CanonicalValue::Null),
            (
                "action_digest",
                CanonicalValue::text(derived.action_digest.as_str()),
            ),
            (
                "state_digest",
                CanonicalValue::text(derived.state_digest.as_str()),
            ),
            (
                "options",
                CanonicalValue::array(
                    APPROVAL_OPTIONS
                        .iter()
                        .map(|value| CanonicalValue::text(*value)),
                ),
            ),
            ("selected_option_id", CanonicalValue::text(selected_option)),
            (
                "requested_capabilities",
                CanonicalValue::array(
                    request
                        .requested_capabilities()
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "created_at",
                CanonicalValue::text(created_at.to_wire_string()),
            ),
            (
                "decided_at",
                CanonicalValue::text(decided_at.to_wire_string()),
            ),
            ("actor_id", CanonicalValue::text(actor)),
        ]))
        .map_err(|error| ApplicationError::application(error.to_string()))?;
        ApprovalRequestRepository::new(&self.database)
            .save(&record)
            .map_err(repository_error)
    }

    #[allow(clippy::too_many_arguments)]
    fn save_checkpoint_approval(
        &self,
        request: &PolicyRequest,
        approval: &WorkflowApproval,
        workflow_id: &str,
        status: ApprovalStatus,
        selected_option: &str,
        actor: &str,
        created_at: &Timestamp,
        decided_at: &Timestamp,
    ) -> ApplicationResult<()> {
        let pending = status == ApprovalStatus::Pending;
        let record = ApprovalRequest::from_value(&CanonicalValue::object([
            (
                "request_id",
                CanonicalValue::text(text_ref(request.request_id())),
            ),
            (
                "checkpoint_type",
                CanonicalValue::text(approval.checkpoint_type().as_str()),
            ),
            ("status", CanonicalValue::text(status.as_str())),
            ("workflow_id", CanonicalValue::text(workflow_id)),
            (
                "step_id",
                CanonicalValue::text(text_ref(approval.step_id())),
            ),
            (
                "action_digest",
                CanonicalValue::text(text_ref(approval.action_digest())),
            ),
            (
                "state_digest",
                CanonicalValue::text(text_ref(approval.state_digest())),
            ),
            (
                "options",
                CanonicalValue::array(
                    APPROVAL_OPTIONS
                        .iter()
                        .map(|value| CanonicalValue::text(*value)),
                ),
            ),
            (
                "selected_option_id",
                if pending {
                    CanonicalValue::Null
                } else {
                    CanonicalValue::text(selected_option)
                },
            ),
            (
                "requested_capabilities",
                CanonicalValue::array(
                    request
                        .requested_capabilities()
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_app_str())),
                ),
            ),
            (
                "created_at",
                CanonicalValue::text(created_at.to_wire_string()),
            ),
            (
                "decided_at",
                if pending {
                    CanonicalValue::Null
                } else {
                    CanonicalValue::text(decided_at.to_wire_string())
                },
            ),
            (
                "actor_id",
                if pending {
                    CanonicalValue::Null
                } else {
                    CanonicalValue::text(actor)
                },
            ),
        ]))
        .map_err(|error| ApplicationError::application(error.to_string()))?;
        ApprovalRequestRepository::new(&self.database)
            .save(&record)
            .map_err(repository_error)
    }

    fn require_stored_decision(
        &self,
        request_id: &str,
        status: ApprovalStatus,
        actor: &str,
    ) -> ApplicationResult<ApprovalRequest> {
        let stored = ApprovalRequestRepository::new(&self.database)
            .get(request_id)
            .map_err(repository_error)?;
        match stored {
            Some(record) if record.status() == status && record.actor_id() == Some(actor) => {
                Ok(record)
            }
            _ => Err(ApplicationError::application(format!(
                "the decision recorded for {request_id} could not be read back"
            ))),
        }
    }

    fn revision_from_prepared(
        &self,
        prepared: &CanonicalValue,
    ) -> ApplicationResult<Option<DesignRevision>> {
        let Some(CanonicalValue::Text(revision_id)) = field(prepared, "revision_id") else {
            return Ok(None);
        };
        DesignRevisionRepository::new(&self.database)
            .get(&json_text_utf8(revision_id))
            .map_err(repository_error)
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn default_capabilities() -> Vec<kala_contracts::Text> {
    let mut capabilities: BTreeSet<String> = PRE_COMMIT_CAPABILITIES
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    for capability in SIMULATOR_CAPABILITIES {
        capabilities.insert(capability.to_owned());
    }
    capabilities.into_iter().map(|value| text(&value)).collect()
}

/// `mechanical_step_catalog`: the declared mechanical step catalog.
///
/// # Errors
///
/// Fails only when a declared step cannot satisfy the catalog contract.
pub fn mechanical_step_catalog() -> ApplicationResult<StepCatalog> {
    let steps = kala_domains::mechanical::MECHANICAL_PACKAGE
        .steps()
        .iter()
        .map(|step| {
            let default_arguments = CanonicalValue::object([
                ("operation", CanonicalValue::text("beam_volume")),
                ("write_report", CanonicalValue::Bool(false)),
            ]);
            let mut keywords: Vec<String> = step.input_names().to_vec();
            keywords.extend(step.metric_names().iter().cloned());
            AgentCatalogStep::from_parts(
                step.step_id(),
                SIMULATOR_TOOL_ID,
                &default_arguments,
                Some(SIMULATOR_TOOL_VERSION),
                &SIMULATOR_CAPABILITIES
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
                &keywords,
                &format!("{}: {}", step.name(), step.description()),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ApplicationError::application(error.to_string()))?;
    StepCatalog::new(steps).map_err(|error| ApplicationError::application(error.to_string()))
}

/// `kernel_state_aggregate`: the internal snapshot aggregate of a kernel
/// state record.
///
/// # Errors
///
/// Refuses an empty aggregate name.
pub fn kernel_state_aggregate(aggregate: &str) -> ApplicationResult<String> {
    if aggregate.is_empty() {
        return Err(ApplicationError::application(
            "a kernel state aggregate must be a nonempty string",
        ));
    }
    Ok(format!("{KERNEL_STATE_NAMESPACE}{aggregate}"))
}

/// `artifact_digest`: the digest an artifact holding this canonical text
/// carries.
#[must_use]
pub fn artifact_digest(payload: &str) -> String {
    sha256_hex_text(&format!("{payload}\n"))
}

fn is_reserved_identifier(value: &str) -> bool {
    reserved_workflow_ids().contains(&value) || value.starts_with(KERNEL_STATE_NAMESPACE)
}

/// `_require_workflow_identifier`.
fn require_workflow_identifier(value: &str) -> ApplicationResult<String> {
    let normalized = require_identifier(value, "workflow_id", MAX_IDENTIFIER_LENGTH)?;
    if is_reserved_identifier(&normalized) {
        return Err(ApplicationError::reserved_identifier(format!(
            "workflow_id is reserved for durable kernel state; it cannot be one of {} or begin \
             with {KERNEL_STATE_NAMESPACE}",
            {
                let mut names = reserved_workflow_ids();
                names.sort_unstable();
                names.join(", ")
            }
        )));
    }
    Ok(normalized)
}

fn text(value: &str) -> kala_contracts::Text {
    kala_contracts::Text::new(value)
}

fn counter_bigint(value: &Counter) -> BigInt {
    BigInt::from_str(&value.to_string()).expect("a counter is a base-ten integer")
}

fn random_hex_12() -> ApplicationResult<String> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApplicationError::application(format!("the local random source is unavailable: {error}"))
    })?;
    let mut out = String::with_capacity(12);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

fn parse_timestamp(value: &str, field_name: &str) -> ApplicationResult<Timestamp> {
    let parsed = kala_kernel::KernelTimestamp::parse_isoformat(value).map_err(|error| {
        let message = if error == kala_kernel::IsoformatError::Naive {
            format!("the recorded {field_name} has no timezone")
        } else {
            format!("the recorded {field_name} is not a timestamp")
        };
        ApplicationError::application(message)
    })?;
    Ok(contract_timestamp(parsed))
}

fn journal_error(error: PersistenceError) -> ApplicationError {
    ApplicationError::application(error.to_string())
}

fn workflow_error(error: kala_kernel::workflow::WorkflowError) -> ApplicationError {
    ApplicationError::application(error.to_string())
}

fn repository_error(error: RepositoryError) -> ApplicationError {
    ApplicationError::application(error.to_string())
}

fn policy_application_error(error: PolicyError) -> ApplicationError {
    ApplicationError::application(error.to_string())
}

fn budget_state_value(value: serde_json::Value) -> ApplicationResult<CanonicalValue> {
    let text = serde_json::to_string(&value)
        .map_err(|error| ApplicationError::application(error.to_string()))?;
    kala_json::parse_strict(text.as_bytes(), &persistence_limits())
        .map_err(|error| ApplicationError::application(error.to_string()))
}

fn package_view(binding: &DomainBinding) -> DomainPackageView {
    let package = binding.package();
    DomainPackageView {
        domain: binding.domain().as_str().to_owned(),
        package_id: package.package_id().to_owned(),
        version: package.version().to_owned(),
        digest: package.digest().to_owned(),
        description: package.description().to_owned(),
        disclaimer: package.disclaimer().to_owned(),
        workflow_definition_id: package.workflow().definition_id().to_owned(),
        reference_subject_id: binding.subject_id().to_owned(),
        schema_digests: package.schema_digests().to_vec(),
        required_capabilities: package.required_capabilities().to_vec(),
        catalog_step_ids: binding.reference_step_ids().to_vec(),
    }
}

fn workflow_view(binding: &DomainBinding) -> WorkflowDefinitionView {
    let definition: &WorkflowDefinition = binding.package().workflow();
    WorkflowDefinitionView {
        domain: binding.domain().as_str().to_owned(),
        definition_id: definition.definition_id().to_owned(),
        version: text_string(definition.version()),
        name: text_string(definition.name()),
        description: definition.description().map(text_string),
        initial_state: definition.initial_state().to_owned(),
        states: definition
            .states()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect(),
        available_step_ids: definition
            .available_step_ids()
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect(),
        transitions: definition
            .transitions()
            .iter()
            .map(|transition| WorkflowTransitionView {
                transition_id: transition.transition_id().to_owned(),
                from_state: transition.from_state().to_owned(),
                to_state: transition.to_state().to_owned(),
                step_id: transition.step_id().to_owned(),
                checkpoint_type: transition.checkpoint_type(),
            })
            .collect(),
        max_replanning_attempts: definition.max_replanning_attempts().as_bigint().clone(),
    }
}

fn approval_view(approval: &WorkflowApproval) -> CheckpointView {
    CheckpointView {
        approval_id: text_string(approval.approval_id()),
        transition_id: text_string(approval.transition_id()),
        step_id: text_string(approval.step_id()),
        checkpoint_type: approval.checkpoint_type(),
        state_digest: text_string(approval.state_digest()),
        action_digest: text_string(approval.action_digest()),
        status: approval.status(),
        actor_id: approval.actor_id().map(text_string),
        decided_at: approval.decided_at().map(contract_timestamp),
        reason: approval.reason().map(text_string),
    }
}

fn state_view(state: &WorkflowState) -> WorkflowStateView {
    WorkflowStateView {
        workflow_id: text_string(state.workflow_id()),
        definition_id: text_string(state.definition_id()),
        definition_version: text_string(state.definition_version()),
        status: state.status(),
        current_state: text_string(state.current_state()),
        completed_step_ids: state.completed_step_ids().iter().map(text_string).collect(),
        attempts: state
            .attempts()
            .iter()
            .map(|attempt: &StepAttempt| StepAttemptView {
                step_id: text_string(attempt.step_id()),
                attempt: attempt.attempt().clone(),
                succeeded: attempt.succeeded(),
                error_code: attempt.error_code().map(text_string),
            })
            .collect(),
        pending_approval: state.pending_approval().map(approval_view),
        replanning_attempts: state.replanning_attempts().clone(),
        history: state
            .history()
            .iter()
            .map(
                |record: &WorkflowTransitionRecord| WorkflowTransitionRecordView {
                    sequence: record.sequence().clone(),
                    transition_id: text_string(record.transition_id()),
                    from_state: text_string(record.from_state()),
                    to_state: text_string(record.to_state()),
                    step_id: text_string(record.step_id()),
                    attempt: record.attempt().clone(),
                },
            )
            .collect(),
        last_error: state.last_error().map(text_string),
    }
}

fn next_step_id(state: &WorkflowState) -> ApplicationResult<String> {
    kala_domains::mechanical::MECHANICAL_WORKFLOW
        .transitions()
        .iter()
        .find(|transition| transition.from_state() == text_ref(state.current_state()))
        .map(|transition| transition.step_id().to_owned())
        .ok_or_else(|| {
            ApplicationError::workflow_finished(format!(
                "workflow state {} declares no outgoing transition",
                text_string(state.current_state())
            ))
        })
}

fn commit_policy_request(derived: &DerivedCommit) -> ApplicationResult<PolicyRequest> {
    PolicyRequest::from_parts(
        text(&derived.request_id),
        text(&derived.state_digest),
        text(&derived.action_digest),
        PRE_COMMIT_CAPABILITIES
            .iter()
            .map(|value| text(value))
            .collect::<Vec<_>>(),
        SideEffectLevel::LocalWrite,
        false,
        false,
        None,
        Some(REVISION_COMMIT_CHECKPOINT),
    )
    .map_err(|error| ApplicationError::checkpoint_decision_denied(error.to_string()))
}

fn policy_request(
    approval: &WorkflowApproval,
    workflow_id: &str,
) -> ApplicationResult<PolicyRequest> {
    PolicyRequest::from_parts(
        text(&format!(
            "policy-{workflow_id}-{}",
            text_ref(approval.transition_id())
        )),
        approval.state_digest().clone(),
        approval.action_digest().clone(),
        PRE_COMMIT_CAPABILITIES
            .iter()
            .map(|value| text(value))
            .collect::<Vec<_>>(),
        SideEffectLevel::LocalWrite,
        false,
        false,
        None,
        Some(approval.checkpoint_type()),
    )
    .map_err(|error| ApplicationError::application(error.to_string()))
}

fn report_payload(
    report: &DomainEvaluationReport,
    parameters: &DomainParameters,
) -> ApplicationResult<CanonicalValue> {
    Ok(CanonicalValue::object([
        (
            "blocked_step_id",
            report
                .blocked_step_id
                .as_ref()
                .map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::text(value.as_str())
                }),
        ),
        (
            "completed_step_ids",
            CanonicalValue::array(
                report
                    .completed_step_ids
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_app_str())),
            ),
        ),
        (
            "disclaimer",
            CanonicalValue::text(report.disclaimer.as_str()),
        ),
        ("domain", CanonicalValue::text(report.domain.as_str())),
        (
            "evaluations",
            CanonicalValue::array(
                report
                    .evaluations
                    .iter()
                    .map(|evaluation| {
                        evaluation
                            .to_canonical_value()
                            .map_err(|error| ApplicationError::application(error.to_string()))
                    })
                    .collect::<ApplicationResult<Vec<_>>>()?,
            ),
        ),
        (
            "findings",
            CanonicalValue::array(
                report
                    .findings
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_app_str())),
            ),
        ),
        (
            "non_authoritative_finding",
            CanonicalValue::text(NON_AUTHORITATIVE_FINDING),
        ),
        (
            "package_digest",
            CanonicalValue::text(report.package_digest.as_str()),
        ),
        (
            "package_id",
            CanonicalValue::text(report.package_id.as_str()),
        ),
        (
            "package_version",
            CanonicalValue::text(report.package_version.as_str()),
        ),
        ("parameters", crate::domains::parameters_value(parameters)?),
        ("passed", CanonicalValue::Bool(report.passed)),
        ("status", CanonicalValue::text(report.status.as_str())),
        (
            "subject_id",
            CanonicalValue::text(report.subject_id.as_str()),
        ),
    ]))
}

fn design_spec(
    binding: &DomainBinding,
    subject_id: &str,
    report: &DomainEvaluationReport,
    parameters: &DomainParameters,
    summary: Option<&str>,
) -> ApplicationResult<DesignSpec> {
    let attributes = CanonicalValue::object([
        ("domain", CanonicalValue::text(binding.domain().as_str())),
        (
            "package_digest",
            CanonicalValue::text(binding.package().digest()),
        ),
        (
            "parameters_digest",
            CanonicalValue::text(report.parameters_digest.as_str()),
        ),
        ("passed", CanonicalValue::Bool(report.passed)),
        ("status", CanonicalValue::text(report.status.as_str())),
        (
            "step_count",
            CanonicalValue::integer(BigInt::from(report.evaluations.len())),
        ),
    ]);
    let attributes_json = canonical_dumps(&attributes, MAX_SPEC_ATTRIBUTES_BYTES)?;
    let resolved_summary = match summary {
        None => format!(
            "{} Bounded surrogate reference evaluation of {subject_id}.",
            binding.package().description()
        ),
        Some(summary) => require_text(summary, "summary", MAX_SUMMARY_LENGTH)?,
    };
    let schema_digest = binding
        .package()
        .schema_digests()
        .first()
        .map_or(CanonicalValue::Null, |value| {
            CanonicalValue::text(value.as_str())
        });
    let _ = parameters;
    DesignSpec::from_value(&CanonicalValue::object([
        ("id", CanonicalValue::text(subject_id)),
        (
            "name",
            CanonicalValue::text(format!("{} reference design", binding.domain().as_str())),
        ),
        ("summary", CanonicalValue::text(resolved_summary)),
        ("parameter_schema_digest", schema_digest),
        ("attributes_json", CanonicalValue::text(attributes_json)),
        ("requirement_ids", CanonicalValue::array([])),
        (
            "constraints",
            CanonicalValue::array([
                CanonicalValue::text(NON_AUTHORITATIVE_FINDING),
                CanonicalValue::text(binding.package().disclaimer()),
            ]),
        ),
    ]))
    .map_err(|error| ApplicationError::application(error.to_string()))
}

fn derived_revision(
    derived: &DerivedCommit,
    created_at: &Timestamp,
) -> ApplicationResult<DesignRevision> {
    DesignRevision::from_value(&CanonicalValue::object([
        (
            "revision_id",
            CanonicalValue::text(derived.revision_id.as_str()),
        ),
        (
            "design_id",
            CanonicalValue::text(derived.design_id.as_str()),
        ),
        (
            "sequence",
            CanonicalValue::integer(derived.sequence.clone()),
        ),
        (
            "spec",
            derived
                .spec
                .to_canonical_value()
                .map_err(|error| ApplicationError::application(error.to_string()))?,
        ),
        (
            "parent_revision_id",
            derived
                .parent_revision_id
                .as_ref()
                .map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::text(value.as_str())
                }),
        ),
        (
            "artifacts",
            CanonicalValue::array([derived
                .artifact
                .to_canonical_value()
                .map_err(|error| ApplicationError::application(error.to_string()))?]),
        ),
        (
            "domain_packages",
            CanonicalValue::array([derived
                .domain_package
                .to_canonical_value()
                .map_err(|error| ApplicationError::application(error.to_string()))?]),
        ),
        (
            "provenance_id",
            CanonicalValue::text(derived.provenance_id.as_str()),
        ),
        (
            "created_at",
            CanonicalValue::text(created_at.to_wire_string()),
        ),
    ]))
    .map_err(|error| ApplicationError::application(error.to_string()))
}

fn derived_provenance(
    derived: &DerivedCommit,
    recorded_at: &Timestamp,
) -> ApplicationResult<Provenance> {
    let mut value = derived
        .provenance_inputs
        .to_canonical_value()
        .map_err(|error| ApplicationError::application(error.to_string()))?;
    if let CanonicalValue::Object(object) = &mut value {
        object.insert(
            JsonText::new("recorded_at"),
            CanonicalValue::text(recorded_at.to_wire_string()),
        );
    }
    Provenance::from_value(&value).map_err(|error| ApplicationError::application(error.to_string()))
}

fn event_view(event: &Event) -> ApplicationResult<(EventView, bool)> {
    let payload = event.payload().map_err(journal_error)?;
    let limits = kala_json::Limits {
        max_json_bytes: MAX_EVENT_PAYLOAD_BYTES,
        ..kala_json::Limits::default()
    };
    let serialized = kala_json::canonical_dumps(&payload, &limits).map_err(|error| {
        ApplicationError::application(format!(
            "the event payload cannot be canonicalized as JSON: {error}"
        ))
    })?;
    let omitted = serialized.len() > MAX_EVENT_PAYLOAD_BYTES;
    Ok((
        EventView {
            sequence: event.sequence,
            event_type: event.event_type.clone(),
            aggregate_id: event.aggregate_id.clone(),
            workflow_id: event.workflow_id.clone(),
            run_id: event.run_id.clone(),
            digest: event.digest.to_hex(),
            occurred_at: contract_timestamp(kernel_from_chrono(event.timestamp)),
            payload: if omitted { None } else { Some(payload) },
            payload_omitted: omitted,
        },
        omitted,
    ))
}

/// Python `str()` of a canonical text value, used only for defensive reads of
/// journalled values.
fn python_text(value: &CanonicalValue) -> ApplicationResult<String> {
    match value {
        CanonicalValue::Text(text) => Ok(json_text_utf8(text)),
        other => canonical_dumps(other, 1_048_576),
    }
}

fn python_optional_text(value: Option<&CanonicalValue>) -> ApplicationResult<Option<String>> {
    match value {
        None | Some(CanonicalValue::Null) => Ok(None),
        Some(value) => python_text(value).map(Some),
    }
}
