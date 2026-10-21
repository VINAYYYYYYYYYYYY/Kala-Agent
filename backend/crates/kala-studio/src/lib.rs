//! Standalone native Kala Studio service.
//!
//! The crate is the Studio front door. It implements the HTTP boundary
//! (loopback Host, same-origin, bearer token, bounded bodies), the settings
//! and token stores, static UI serving, the deterministic local chat
//! provider and the fixed-endpoint remote OpenAI-compatible provider.
//!
//! # The geometry front door (ADR-0010, ADR-0011)
//!
//! Geometry is never re-implemented in Rust. The front door supervises one
//! Python geometry worker ([`worker`]) and proxies the geometry, design,
//! journal, intent and import routes to it ([`geometry_backend`]). The
//! routes it answers itself are `/health`, `/integrations`, `/providers`,
//! `/settings` and `/chat`; `/chat` may carry a design, which the front
//! door orchestrates through the worker ([`chat_design`]).
//!
//! The reviewed native geometry crates are parked behind the non-default
//! `native-primitives` feature ([`native_backend`]). A default build links
//! no native geometry crate at all and never fabricates a mesh, a STEP file
//! or a measurement.
//!
//! JSON floats use `kala-json`'s CPython formatter.

#![forbid(unsafe_code)]

pub mod artifacts;
pub mod boundary;
pub mod chat_design;
pub mod cli;
pub mod errors;
pub mod geometry;
pub mod geometry_backend;
pub mod integrations;
pub mod json;
pub mod json_diagnostics;
pub mod json_input;
#[cfg(feature = "native-primitives")]
pub mod native_backend;
#[cfg(feature = "native-geometry")]
pub mod native_geometry;
pub mod providers;
pub mod proxy;
pub mod pytext;
pub mod schemas;
pub mod server;
pub mod settings;
pub mod settings_json;
pub mod static_files;
pub mod step_limit;
pub mod tls;
pub mod worker;

pub use geometry::{StepCapability, StepError, StepExporter};
pub use geometry_backend::{BackendError, BackendHealth, GeometryBackend, WorkerResponse};
pub use server::{StartupError, StudioConfig, StudioService};
pub use worker::{GeometryWorker, WorkerCommand, WorkerState};
