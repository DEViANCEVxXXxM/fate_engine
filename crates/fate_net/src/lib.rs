//! Transport-agnostic local protocol prototype built on `fate_sim` snapshots.
//!
//! This crate encodes versioned client-input and server-snapshot packets and
//! provides client/server session state machines. It does not open sockets,
//! authenticate peers, encrypt data, or implement production server policy.
//!
//! Trust boundaries are explicit:
//!
//! - Client input is untrusted bytes. The server validates packet structure and
//!   ordering, then applies the payload through a caller-supplied function that
//!   must validate application commands.
//! - Server snapshots are untrusted until `SimulationState::from_state_bytes`
//!   accepts them and the packet tick matches the encoded state tick. State
//!   hashes are diagnostic checksums only, never authentication.
//! - Payload sizes and acknowledgement history are bounded by [`Limits`].

#![forbid(unsafe_code)]

pub mod protocol;
pub mod session;

pub use protocol::{Limits, PROTOCOL_VERSION, Packet, ProtocolError};
pub use session::{
    ApplyResult, ClientDiagnostics, ClientError, ClientSession, ServerDiagnostics, ServerError,
    ServerSession, SnapshotInfo,
};
