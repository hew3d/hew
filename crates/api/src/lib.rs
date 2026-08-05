//! The Hew API's logical command bus (docs/HEW_API.md — the normative
//! reference). One pure dispatcher between the kernel and its hosts:
//! transports (in-process, local socket, stdio MCP) are dumb pipes that
//! carry serialized envelopes here and responses back.
//!
//! This crate owns everything contractual: the JSON-RPC 2.0 envelope
//! (§4), the command registry and its published schemas (§9), profile
//! enforcement (§10), transaction execution (§6), and the api-owned
//! public-id indirection (§5.1). It follows the kernel's purity rule
//! (DEVELOPMENT.md rule 1): no UI, no I/O, no network — host effects
//! (file paths, importers, rendering) are delegated through a typed host
//! trait a host implements to its ability.
//!
//! Status: scaffold. The registry declares the protocol-1 command
//! inventory; every command not yet implemented answers the
//! `unimplemented` refusal (§14 — the declared-but-unimplemented set is
//! the burn-down list, and the conformance suite's tests for a command
//! land before its implementation does).

pub mod codegen;
pub mod commands;
pub mod dispatch;
pub mod envelope;
pub mod geom;
pub mod host;
pub mod ids;
pub mod locate;
pub mod refusal;
pub mod registry;
pub mod transact;

pub use dispatch::{Connection, DispatchOutcome};
pub use envelope::{ErrorObject, Request, RequestId, Response, codes};
pub use host::{
    Host, NoHost, SnapshotCamera, SnapshotParams, SnapshotProjection, SnapshotResult, StandardView,
    ViewCameraSpec,
};
pub use ids::IdResolver;
pub use refusal::Refusal;
pub use registry::{CommandClass, CommandDecl, Profile, Registry, Served, Tier};
