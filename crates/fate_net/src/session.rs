//! Client and server session state machines built on the packet protocol.
//!
//! The server applies one ordered input per simulation tick and keeps a bounded
//! acknowledgement history. The client tracks the highest acknowledged input
//! and validates every snapshot before accepting it. Neither side establishes
//! peer identity or trust.

use std::collections::VecDeque;
use std::fmt;

use fate_sim::simulation::{SimulationDiagnostics, SimulationState, SimulationStateError};

use crate::protocol::{Limits, Packet, ProtocolError};

/// Result returned by the caller-supplied server input applier.
pub type ApplyResult = Result<(), String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerError {
    Protocol(ProtocolError),
    UnexpectedPacket,
    UnexpectedTick { expected: u64, received: u64 },
    Apply(String),
    InvalidSnapshot(SimulationStateError),
    TickOverflow,
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "protocol error: {error}"),
            Self::UnexpectedPacket => formatter.write_str("expected an input packet"),
            Self::UnexpectedTick { expected, received } => {
                write!(
                    formatter,
                    "expected client tick {expected}, received {received}"
                )
            }
            Self::Apply(error) => write!(formatter, "input rejected: {error}"),
            Self::InvalidSnapshot(error) => write!(formatter, "invalid local snapshot: {error}"),
            Self::TickOverflow => formatter.write_str("simulation tick counter is exhausted"),
        }
    }
}

impl std::error::Error for ServerError {}

pub struct ServerSession {
    limits: Limits,
    state: SimulationState,
    applied_inputs: VecDeque<u64>,
    next_client_tick: u64,
}

/// Read-only snapshot of server session state for tooling and diagnostics.
///
/// `simulation.state_hash` is a diagnostic checksum, not authentication.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerDiagnostics {
    pub server_tick: u64,
    pub next_client_tick: u64,
    pub last_acked_client_tick: Option<u64>,
    pub acked_history_len: usize,
    pub max_ack_history: usize,
    pub simulation: SimulationDiagnostics,
}

impl ServerSession {
    pub fn new(limits: Limits) -> Result<Self, ServerError> {
        Self::with_state(limits, SimulationState::new())
    }

    /// Start from an existing deterministic state, e.g. after restoring a
    /// snapshot. The next accepted client tick is `state.tick() + 1`, and prior
    /// ticks are treated as acknowledged.
    pub fn with_state(limits: Limits, state: SimulationState) -> Result<Self, ServerError> {
        limits.validate().map_err(ServerError::Protocol)?;
        let next_client_tick = state
            .tick()
            .checked_add(1)
            .ok_or(ServerError::TickOverflow)?;
        let mut applied_inputs = VecDeque::new();
        if state.tick() > 0 {
            applied_inputs.push_back(state.tick());
        }
        Ok(Self {
            limits,
            state,
            applied_inputs,
            next_client_tick,
        })
    }

    pub fn state(&self) -> &SimulationState {
        &self.state
    }

    pub fn server_tick(&self) -> u64 {
        self.state.tick()
    }

    pub fn last_acked_client_tick(&self) -> Option<u64> {
        self.applied_inputs.back().copied()
    }

    pub fn acked_history_len(&self) -> usize {
        self.applied_inputs.len()
    }

    /// Capture a read-only snapshot of session and simulation state.
    pub fn diagnostics(&self) -> ServerDiagnostics {
        ServerDiagnostics {
            server_tick: self.state.tick(),
            next_client_tick: self.next_client_tick,
            last_acked_client_tick: self.last_acked_client_tick(),
            acked_history_len: self.applied_inputs.len(),
            max_ack_history: self.limits.max_ack_history,
            simulation: self.state.diagnostics(),
        }
    }

    /// Decode and validate an input packet, then apply it transactionally.
    ///
    /// Inputs must arrive in order starting at client tick 1; the transport is
    /// responsible for retransmission. A gap, duplicate, or stale tick is
    /// rejected with [`ServerError::UnexpectedTick`]. The input is committed
    /// only when `apply` succeeds. On any error the server state, tick, and
    /// acknowledgement history are unchanged.
    pub fn accept_input<F>(&mut self, bytes: &[u8], apply: F) -> Result<u64, ServerError>
    where
        F: FnOnce(&mut SimulationState, &[u8]) -> ApplyResult,
    {
        let packet = Packet::decode(bytes, &self.limits).map_err(ServerError::Protocol)?;
        let Packet::Input { client_tick, input } = packet else {
            return Err(ServerError::UnexpectedPacket);
        };

        let expected = self.next_client_tick;
        if client_tick != expected {
            return Err(ServerError::UnexpectedTick {
                expected,
                received: client_tick,
            });
        }

        let mut candidate = SimulationState::from_state_bytes(&self.state.state_bytes())
            .map_err(ServerError::InvalidSnapshot)?;
        apply(&mut candidate, &input).map_err(ServerError::Apply)?;
        candidate
            .advance_tick()
            .map_err(|_| ServerError::TickOverflow)?;

        self.state = candidate;
        self.next_client_tick = client_tick
            .checked_add(1)
            .ok_or(ServerError::TickOverflow)?;
        self.applied_inputs.push_back(client_tick);
        while self.applied_inputs.len() > self.limits.max_ack_history {
            self.applied_inputs.pop_front();
        }
        Ok(client_tick)
    }

    pub fn snapshot(&self) -> Packet {
        Packet::Snapshot {
            server_tick: self.state.tick(),
            acked_client_tick: self.applied_inputs.back().copied().unwrap_or(0),
            state_hash: self.state.state_hash(),
            state: self.state.state_bytes(),
        }
    }

    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        self.snapshot().encode(&self.limits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientError {
    Protocol(ProtocolError),
    UnexpectedPacket,
    InvalidSnapshot(SimulationStateError),
    InvalidStartTick,
    TickMismatch { packet: u64, state: u64 },
    AckAheadOfInput { acked: u64, highest_sent: u64 },
    StaleAck { previous: u64, received: u64 },
    StaleServerTick { previous: u64, received: u64 },
    TickOverflow,
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "protocol error: {error}"),
            Self::UnexpectedPacket => formatter.write_str("expected a snapshot packet"),
            Self::InvalidSnapshot(error) => write!(formatter, "invalid snapshot: {error}"),
            Self::InvalidStartTick => formatter.write_str("next input tick must be at least 1"),
            Self::TickMismatch { packet, state } => write!(
                formatter,
                "snapshot server tick {packet} does not match encoded state tick {state}"
            ),
            Self::AckAheadOfInput {
                acked,
                highest_sent,
            } => write!(
                formatter,
                "server acknowledged input tick {acked} but only {highest_sent} were sent"
            ),
            Self::StaleAck { previous, received } => {
                write!(
                    formatter,
                    "stale acknowledgement {received}; previous was {previous}"
                )
            }
            Self::StaleServerTick { previous, received } => {
                write!(
                    formatter,
                    "stale server tick {received}; previous was {previous}"
                )
            }
            Self::TickOverflow => formatter.write_str("input tick counter is exhausted"),
        }
    }
}

impl std::error::Error for ClientError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub server_tick: u64,
    pub acked_client_tick: u64,
    pub state_hash: u64,
    pub state: Vec<u8>,
}

pub struct ClientSession {
    limits: Limits,
    next_input_tick: u64,
    last_acked_input_tick: u64,
    last_server_tick: u64,
    has_snapshot: bool,
}

/// Read-only snapshot of client session state for tooling and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientDiagnostics {
    pub next_input_tick: u64,
    pub last_acked_input_tick: u64,
    pub last_server_tick: u64,
    pub has_snapshot: bool,
}

impl ClientSession {
    pub fn new(limits: Limits) -> Result<Self, ClientError> {
        Self::with_next_input_tick(limits, 1)
    }

    /// Start numbering inputs at `next_input_tick`, e.g. after restoring a
    /// snapshot. Ticks below `next_input_tick` are treated as already
    /// acknowledged.
    pub fn with_next_input_tick(limits: Limits, next_input_tick: u64) -> Result<Self, ClientError> {
        limits.validate().map_err(ClientError::Protocol)?;
        let last_acked_input_tick = next_input_tick
            .checked_sub(1)
            .ok_or(ClientError::InvalidStartTick)?;
        Ok(Self {
            limits,
            next_input_tick,
            last_acked_input_tick,
            last_server_tick: 0,
            has_snapshot: false,
        })
    }

    pub fn next_input_tick(&self) -> u64 {
        self.next_input_tick
    }

    pub fn last_acked_input_tick(&self) -> u64 {
        self.last_acked_input_tick
    }

    pub fn last_server_tick(&self) -> u64 {
        self.last_server_tick
    }

    /// Capture a read-only snapshot of client session state.
    pub fn diagnostics(&self) -> ClientDiagnostics {
        ClientDiagnostics {
            next_input_tick: self.next_input_tick,
            last_acked_input_tick: self.last_acked_input_tick,
            last_server_tick: self.last_server_tick,
            has_snapshot: self.has_snapshot,
        }
    }

    /// Encode an input for the next client tick and advance the tick counter.
    pub fn make_input(&mut self, input: Vec<u8>) -> Result<Vec<u8>, ClientError> {
        let tick = self.next_input_tick;
        let bytes = Packet::Input {
            client_tick: tick,
            input,
        }
        .encode(&self.limits)
        .map_err(ClientError::Protocol)?;
        self.next_input_tick = tick.checked_add(1).ok_or(ClientError::TickOverflow)?;
        Ok(bytes)
    }

    /// Validate and accept a server snapshot.
    ///
    /// The packet's `server_tick` must equal the tick encoded in the snapshot
    /// state. The session is unchanged on error. The returned snapshot has
    /// already passed `SimulationState::from_state_bytes`, but callers must
    /// still treat its contents as untrusted application data.
    pub fn receive_snapshot(&mut self, bytes: &[u8]) -> Result<SnapshotInfo, ClientError> {
        let packet = Packet::decode(bytes, &self.limits).map_err(ClientError::Protocol)?;
        let Packet::Snapshot {
            server_tick,
            acked_client_tick,
            state_hash,
            state,
        } = packet
        else {
            return Err(ClientError::UnexpectedPacket);
        };

        // Trust boundary: validate the schema and invariants before accepting
        // any field from the packet.
        let restored =
            SimulationState::from_state_bytes(&state).map_err(ClientError::InvalidSnapshot)?;
        let actual = restored.state_hash();
        if actual != state_hash {
            return Err(ClientError::Protocol(ProtocolError::HashMismatch {
                expected: state_hash,
                actual,
            }));
        }
        let state_tick = restored.tick();
        if state_tick != server_tick {
            return Err(ClientError::TickMismatch {
                packet: server_tick,
                state: state_tick,
            });
        }

        let highest_sent = self.next_input_tick.saturating_sub(1);
        if acked_client_tick > highest_sent {
            return Err(ClientError::AckAheadOfInput {
                acked: acked_client_tick,
                highest_sent,
            });
        }
        if acked_client_tick < self.last_acked_input_tick {
            return Err(ClientError::StaleAck {
                previous: self.last_acked_input_tick,
                received: acked_client_tick,
            });
        }
        if self.has_snapshot && server_tick < self.last_server_tick {
            return Err(ClientError::StaleServerTick {
                previous: self.last_server_tick,
                received: server_tick,
            });
        }

        self.last_acked_input_tick = acked_client_tick;
        self.last_server_tick = server_tick;
        self.has_snapshot = true;
        Ok(SnapshotInfo {
            server_tick,
            acked_client_tick,
            state_hash,
            state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fate_sim::{StateHasher, StateWriter};

    fn apply_nothing(_: &mut SimulationState, _: &[u8]) -> ApplyResult {
        Ok(())
    }

    fn apply_rng(state: &mut SimulationState, input: &[u8]) -> ApplyResult {
        let steps = input.first().copied().unwrap_or(0) as u32;
        for _ in 0..steps {
            let _ = state.random_u64();
        }
        Ok(())
    }

    fn hash(bytes: &[u8]) -> u64 {
        let mut hasher = StateHasher::new();
        hasher.write_bytes(bytes);
        hasher.finish()
    }

    #[test]
    fn server_rejects_out_of_order_inputs_without_changing_state() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();
        let first = client.make_input(vec![1]).unwrap();
        let second = client.make_input(vec![1]).unwrap();

        assert_eq!(
            server.accept_input(&second, apply_rng),
            Err(ServerError::UnexpectedTick {
                expected: 1,
                received: 2
            })
        );
        assert_eq!(server.server_tick(), 0);
        assert_eq!(server.last_acked_client_tick(), None);

        assert_eq!(server.accept_input(&first, apply_rng).unwrap(), 1);
        assert_eq!(server.server_tick(), 1);
    }

    #[test]
    fn server_apply_failure_is_transactional() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();
        let input = client.make_input(vec![1]).unwrap();
        let before = server.state().state_bytes();

        assert_eq!(
            server.accept_input(&input, |_, _| Err("bad command".to_owned())),
            Err(ServerError::Apply("bad command".to_owned()))
        );
        assert_eq!(server.state().state_bytes(), before);
        assert_eq!(server.server_tick(), 0);
        assert_eq!(server.last_acked_client_tick(), None);

        assert_eq!(server.accept_input(&input, apply_rng).unwrap(), 1);
    }

    #[test]
    fn server_ack_history_is_bounded() {
        let limits = Limits {
            max_ack_history: 3,
            ..Limits::default()
        };
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();

        for _ in 0..10 {
            let input = client.make_input(vec![0]).unwrap();
            server.accept_input(&input, apply_nothing).unwrap();
        }

        assert_eq!(server.acked_history_len(), 3);
        assert_eq!(server.last_acked_client_tick(), Some(10));
    }

    #[test]
    fn server_rejects_snapshot_packets() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let snapshot = server.snapshot_bytes().unwrap();
        assert_eq!(
            server.accept_input(&snapshot, apply_nothing),
            Err(ServerError::UnexpectedPacket)
        );
    }

    #[test]
    fn client_rejects_stale_future_and_ahead_acks() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();
        let input = client.make_input(vec![1]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();
        let accepted = server.snapshot();
        client
            .receive_snapshot(&accepted.encode(&limits).unwrap())
            .unwrap();

        let Packet::Snapshot {
            server_tick,
            acked_client_tick,
            state_hash,
            state,
        } = accepted
        else {
            panic!("expected snapshot");
        };

        let ahead = Packet::Snapshot {
            server_tick,
            acked_client_tick: acked_client_tick + 1,
            state_hash,
            state: state.clone(),
        };
        assert_eq!(
            client.receive_snapshot(&ahead.encode(&limits).unwrap()),
            Err(ClientError::AckAheadOfInput {
                acked: acked_client_tick + 1,
                highest_sent: acked_client_tick,
            })
        );

        let stale = Packet::Snapshot {
            server_tick,
            acked_client_tick: acked_client_tick - 1,
            state_hash,
            state: state.clone(),
        };
        assert_eq!(
            client.receive_snapshot(&stale.encode(&limits).unwrap()),
            Err(ClientError::StaleAck {
                previous: acked_client_tick,
                received: acked_client_tick - 1,
            })
        );

        // A stale server tick must carry a matching state tick; otherwise the
        // tick-consistency check rejects it first.
        let older = SimulationState::new();
        let stale_tick = Packet::Snapshot {
            server_tick: 0,
            acked_client_tick,
            state_hash: older.state_hash(),
            state: older.state_bytes(),
        };
        assert_eq!(
            client.receive_snapshot(&stale_tick.encode(&limits).unwrap()),
            Err(ClientError::StaleServerTick {
                previous: server_tick,
                received: 0,
            })
        );
    }

    #[test]
    fn client_rejects_invalid_snapshots_and_wrong_packets() {
        let limits = Limits::default();
        let mut client = ClientSession::new(limits).unwrap();

        let input = Packet::Input {
            client_tick: 1,
            input: Vec::new(),
        };
        assert_eq!(
            client.receive_snapshot(&input.encode(&limits).unwrap()),
            Err(ClientError::UnexpectedPacket)
        );

        let bad_state = vec![0_u8; 8];
        let invalid = Packet::Snapshot {
            server_tick: 1,
            acked_client_tick: 0,
            state_hash: hash(&bad_state),
            state: bad_state,
        };
        assert!(matches!(
            client.receive_snapshot(&invalid.encode(&limits).unwrap()),
            Err(ClientError::InvalidSnapshot(_))
        ));

        let mut tampered = Packet::Snapshot {
            server_tick: 1,
            acked_client_tick: 0,
            state_hash: hash(&[1, 2, 3]),
            state: vec![1, 2, 3],
        }
        .encode(&limits)
        .unwrap();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(matches!(
            client.receive_snapshot(&tampered),
            Err(ClientError::Protocol(ProtocolError::HashMismatch { .. }))
        ));
    }

    #[test]
    fn client_rejects_oversized_input_without_advancing_tick() {
        let limits = Limits {
            max_input_bytes: 2,
            ..Limits::default()
        };
        let mut client = ClientSession::new(limits).unwrap();
        assert!(matches!(
            client.make_input(vec![0; 3]),
            Err(ClientError::Protocol(ProtocolError::PayloadTooLarge { .. }))
        ));
        assert_eq!(client.next_input_tick(), 1);
    }

    #[test]
    fn client_rejects_snapshot_tick_that_disagrees_with_state() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();
        let input = client.make_input(vec![1]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();

        let Packet::Snapshot {
            server_tick,
            acked_client_tick,
            state_hash,
            state,
        } = server.snapshot()
        else {
            panic!("expected snapshot");
        };

        for claimed in [server_tick + 1, server_tick.saturating_sub(1)] {
            let mismatched = Packet::Snapshot {
                server_tick: claimed,
                acked_client_tick,
                state_hash,
                state: state.clone(),
            };
            assert_eq!(
                client.receive_snapshot(&mismatched.encode(&limits).unwrap()),
                Err(ClientError::TickMismatch {
                    packet: claimed,
                    state: server_tick,
                })
            );
        }

        assert_eq!(client.last_server_tick(), 0);
        assert_eq!(client.last_acked_input_tick(), 0);
    }

    #[test]
    fn client_recovers_after_rejecting_an_inconsistent_snapshot() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();

        let input = client.make_input(vec![1]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();
        let accepted = client
            .receive_snapshot(&server.snapshot_bytes().unwrap())
            .unwrap();
        assert_eq!(accepted.server_tick, 1);
        assert_eq!(client.last_server_tick(), 1);
        assert_eq!(client.last_acked_input_tick(), 1);

        let Packet::Snapshot {
            server_tick,
            acked_client_tick,
            state_hash,
            state,
        } = server.snapshot()
        else {
            panic!("expected snapshot");
        };
        let inconsistent = Packet::Snapshot {
            server_tick: server_tick + 2,
            acked_client_tick,
            state_hash,
            state,
        };
        assert!(matches!(
            client.receive_snapshot(&inconsistent.encode(&limits).unwrap()),
            Err(ClientError::TickMismatch { .. })
        ));
        assert_eq!(client.last_server_tick(), 1);
        assert_eq!(client.last_acked_input_tick(), 1);

        let input = client.make_input(vec![1]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();
        let accepted = client
            .receive_snapshot(&server.snapshot_bytes().unwrap())
            .unwrap();
        assert_eq!(accepted.server_tick, 2);
        assert_eq!(client.last_server_tick(), 2);
        assert_eq!(client.last_acked_input_tick(), 2);
    }

    #[test]
    fn server_and_client_resume_from_an_aligned_tick() {
        let limits = Limits::default();
        let mut state = SimulationState::with_seed(99);
        for _ in 0..5 {
            let _ = state.random_u64();
            state.advance_tick().unwrap();
        }

        let mut server = ServerSession::with_state(limits, state).unwrap();
        assert_eq!(server.server_tick(), 5);
        assert_eq!(server.last_acked_client_tick(), Some(5));

        let mut client = ClientSession::with_next_input_tick(limits, 6).unwrap();
        assert_eq!(client.last_acked_input_tick(), 5);

        let input = client.make_input(vec![1]).unwrap();
        assert_eq!(server.accept_input(&input, apply_rng).unwrap(), 6);
        assert_eq!(server.server_tick(), 6);
    }

    #[test]
    fn client_rejects_zero_start_tick() {
        assert_eq!(
            ClientSession::with_next_input_tick(Limits::default(), 0).err(),
            Some(ClientError::InvalidStartTick)
        );
    }

    #[test]
    fn server_diagnostics_track_ticks_ack_history_and_simulation() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();

        let initial = server.diagnostics();
        assert_eq!(initial.server_tick, 0);
        assert_eq!(initial.next_client_tick, 1);
        assert_eq!(initial.last_acked_client_tick, None);
        assert_eq!(initial.acked_history_len, 0);
        assert_eq!(initial.max_ack_history, limits.max_ack_history);
        assert_eq!(initial.simulation.state_hash, server.state().state_hash());

        let input = client.make_input(vec![2]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();

        let updated = server.diagnostics();
        assert_eq!(updated.server_tick, 1);
        assert_eq!(updated.next_client_tick, 2);
        assert_eq!(updated.last_acked_client_tick, Some(1));
        assert_eq!(updated.acked_history_len, 1);
        assert_eq!(updated.simulation.tick, 1);
        assert_eq!(updated.simulation.state_hash, server.state().state_hash());
    }

    #[test]
    fn client_diagnostics_track_input_and_snapshot_progress() {
        let limits = Limits::default();
        let mut server = ServerSession::new(limits).unwrap();
        let mut client = ClientSession::new(limits).unwrap();

        let initial = client.diagnostics();
        assert_eq!(initial.next_input_tick, 1);
        assert_eq!(initial.last_acked_input_tick, 0);
        assert_eq!(initial.last_server_tick, 0);
        assert!(!initial.has_snapshot);

        let input = client.make_input(vec![1]).unwrap();
        server.accept_input(&input, apply_rng).unwrap();
        client
            .receive_snapshot(&server.snapshot_bytes().unwrap())
            .unwrap();

        let updated = client.diagnostics();
        assert_eq!(updated.next_input_tick, 2);
        assert_eq!(updated.last_acked_input_tick, 1);
        assert_eq!(updated.last_server_tick, 1);
        assert!(updated.has_snapshot);
    }
}
