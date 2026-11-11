//! Canonical little-endian packet encoding and resource limits.
//!
//! Packets are transport-agnostic byte buffers. Decoding validates structure,
//! versions, declared lengths, and snapshot checksums before allocating or
//! accepting payload data.

use std::fmt;

use fate_sim::{StateHasher, StateWriter};

/// Current protocol schema version.
pub const PROTOCOL_VERSION: u16 = 1;

const MAGIC: [u8; 4] = *b"FNET";
const KIND_INPUT: u8 = 0;
const KIND_SNAPSHOT: u8 = 1;
const MAX_PAYLOAD_BYTES: usize = u32::MAX as usize;

/// Resource caps applied when encoding or decoding packets.
///
/// `max_snapshot_bytes` must be large enough for the consumer's simulation
/// state. All limits must be non-zero and fit in a 32-bit length field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_snapshot_bytes: usize,
    pub max_ack_history: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64,
            max_snapshot_bytes: 1024 * 1024,
            max_ack_history: 256,
        }
    }
}

impl Limits {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.max_input_bytes == 0 || self.max_input_bytes > MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::InvalidLimits("max_input_bytes"));
        }
        if self.max_snapshot_bytes == 0 || self.max_snapshot_bytes > MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::InvalidLimits("max_snapshot_bytes"));
        }
        if self.max_ack_history == 0 {
            return Err(ProtocolError::InvalidLimits("max_ack_history"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    BadMagic,
    UnsupportedVersion(u16),
    UnknownPacket(u8),
    Truncated,
    TrailingBytes,
    InvalidLimits(&'static str),
    PayloadTooLarge {
        kind: &'static str,
        size: usize,
        limit: usize,
    },
    HashMismatch {
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => formatter.write_str("invalid protocol signature"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::UnknownPacket(kind) => write!(formatter, "unknown packet kind {kind}"),
            Self::Truncated => formatter.write_str("truncated packet"),
            Self::TrailingBytes => formatter.write_str("unexpected trailing packet bytes"),
            Self::InvalidLimits(name) => write!(formatter, "invalid protocol limit: {name}"),
            Self::PayloadTooLarge { kind, size, limit } => {
                write!(formatter, "{kind} payload {size} exceeds limit {limit}")
            }
            Self::HashMismatch { expected, actual } => write!(
                formatter,
                "snapshot hash mismatch: expected {expected:016x}, got {actual:016x}"
            ),
        }
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    Input {
        client_tick: u64,
        input: Vec<u8>,
    },
    Snapshot {
        server_tick: u64,
        acked_client_tick: u64,
        state_hash: u64,
        state: Vec<u8>,
    },
}

impl Packet {
    pub fn encode(&self, limits: &Limits) -> Result<Vec<u8>, ProtocolError> {
        limits.validate()?;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());

        match self {
            Self::Input { client_tick, input } => {
                if input.len() > limits.max_input_bytes {
                    return Err(ProtocolError::PayloadTooLarge {
                        kind: "input",
                        size: input.len(),
                        limit: limits.max_input_bytes,
                    });
                }
                bytes.push(KIND_INPUT);
                bytes.extend_from_slice(&client_tick.to_le_bytes());
                bytes.extend_from_slice(&payload_len(input.len()).to_le_bytes());
                bytes.extend_from_slice(input);
            }
            Self::Snapshot {
                server_tick,
                acked_client_tick,
                state_hash,
                state,
            } => {
                if state.len() > limits.max_snapshot_bytes {
                    return Err(ProtocolError::PayloadTooLarge {
                        kind: "snapshot",
                        size: state.len(),
                        limit: limits.max_snapshot_bytes,
                    });
                }
                bytes.push(KIND_SNAPSHOT);
                bytes.extend_from_slice(&server_tick.to_le_bytes());
                bytes.extend_from_slice(&acked_client_tick.to_le_bytes());
                bytes.extend_from_slice(&state_hash.to_le_bytes());
                bytes.extend_from_slice(&payload_len(state.len()).to_le_bytes());
                bytes.extend_from_slice(state);
            }
        }

        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, ProtocolError> {
        limits.validate()?;

        let mut offset = 0;
        if read::<4>(bytes, &mut offset)? != MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        let version = u16::from_le_bytes(read(bytes, &mut offset)?);
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }

        let packet = match read::<1>(bytes, &mut offset)?[0] {
            KIND_INPUT => {
                let client_tick = u64::from_le_bytes(read(bytes, &mut offset)?);
                let len = u32::from_le_bytes(read(bytes, &mut offset)?) as usize;
                if len > limits.max_input_bytes {
                    return Err(ProtocolError::PayloadTooLarge {
                        kind: "input",
                        size: len,
                        limit: limits.max_input_bytes,
                    });
                }
                let input = take(bytes, &mut offset, len)?.to_vec();
                Self::Input { client_tick, input }
            }
            KIND_SNAPSHOT => {
                let server_tick = u64::from_le_bytes(read(bytes, &mut offset)?);
                let acked_client_tick = u64::from_le_bytes(read(bytes, &mut offset)?);
                let state_hash = u64::from_le_bytes(read(bytes, &mut offset)?);
                let len = u32::from_le_bytes(read(bytes, &mut offset)?) as usize;
                if len > limits.max_snapshot_bytes {
                    return Err(ProtocolError::PayloadTooLarge {
                        kind: "snapshot",
                        size: len,
                        limit: limits.max_snapshot_bytes,
                    });
                }
                let state = take(bytes, &mut offset, len)?.to_vec();
                let actual = fnv1a(&state);
                if actual != state_hash {
                    return Err(ProtocolError::HashMismatch {
                        expected: state_hash,
                        actual,
                    });
                }
                Self::Snapshot {
                    server_tick,
                    acked_client_tick,
                    state_hash,
                    state,
                }
            }
            other => return Err(ProtocolError::UnknownPacket(other)),
        };

        if offset != bytes.len() {
            return Err(ProtocolError::TrailingBytes);
        }
        Ok(packet)
    }
}

fn payload_len(len: usize) -> u32 {
    // `Limits::validate` rejects limits above `u32::MAX`, and encode checks the
    // payload against the limit before calling this.
    u32::try_from(len).expect("payload length checked against u32 limit")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hasher = StateHasher::new();
    hasher.write_bytes(bytes);
    hasher.finish()
}

fn read<const N: usize>(bytes: &[u8], offset: &mut usize) -> Result<[u8; N], ProtocolError> {
    let end = offset.checked_add(N).ok_or(ProtocolError::Truncated)?;
    let value = bytes.get(*offset..end).ok_or(ProtocolError::Truncated)?;
    *offset = end;
    Ok(value.try_into().expect("fixed-size slice"))
}

fn take<'a>(bytes: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], ProtocolError> {
    let end = offset.checked_add(len).ok_or(ProtocolError::Truncated)?;
    let value = bytes.get(*offset..end).ok_or(ProtocolError::Truncated)?;
    *offset = end;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> Packet {
        Packet::Input {
            client_tick: 42,
            input: vec![1, 2, 3, 4],
        }
    }

    fn sample_snapshot() -> Packet {
        let state = vec![9, 8, 7, 6, 5];
        Packet::Snapshot {
            server_tick: 7,
            acked_client_tick: 6,
            state_hash: fnv1a(&state),
            state,
        }
    }

    #[test]
    fn packets_round_trip() {
        let limits = Limits::default();
        for packet in [sample_input(), sample_snapshot()] {
            let bytes = packet.encode(&limits).unwrap();
            assert_eq!(Packet::decode(&bytes, &limits).unwrap(), packet);
        }
    }

    #[test]
    fn decoder_rejects_every_truncation() {
        let limits = Limits::default();
        for packet in [sample_input(), sample_snapshot()] {
            let bytes = packet.encode(&limits).unwrap();
            for length in 0..bytes.len() {
                assert!(
                    Packet::decode(&bytes[..length], &limits).is_err(),
                    "prefix {length} of {bytes:?} decoded"
                );
            }
        }
    }

    #[test]
    fn decoder_rejects_bad_magic_version_kind_and_trailing_bytes() {
        let limits = Limits::default();
        let bytes = sample_input().encode(&limits).unwrap();

        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 1;
        assert_eq!(
            Packet::decode(&bad_magic, &limits),
            Err(ProtocolError::BadMagic)
        );

        let mut bad_version = bytes.clone();
        bad_version[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            Packet::decode(&bad_version, &limits),
            Err(ProtocolError::UnsupportedVersion(u16::MAX))
        );

        let mut bad_kind = bytes.clone();
        bad_kind[6] = 0xff;
        assert_eq!(
            Packet::decode(&bad_kind, &limits),
            Err(ProtocolError::UnknownPacket(0xff))
        );

        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(
            Packet::decode(&trailing, &limits),
            Err(ProtocolError::TrailingBytes)
        );
    }

    #[test]
    fn payload_limits_are_enforced_on_encode_and_decode() {
        let limits = Limits {
            max_input_bytes: 2,
            ..Limits::default()
        };
        let oversized = Packet::Input {
            client_tick: 1,
            input: vec![0; 3],
        };
        assert!(matches!(
            oversized.encode(&limits),
            Err(ProtocolError::PayloadTooLarge { .. })
        ));

        let mut declared_too_large = Vec::new();
        declared_too_large.extend_from_slice(&MAGIC);
        declared_too_large.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        declared_too_large.push(KIND_INPUT);
        declared_too_large.extend_from_slice(&1u64.to_le_bytes());
        declared_too_large.extend_from_slice(&3u32.to_le_bytes());
        assert!(matches!(
            Packet::decode(&declared_too_large, &limits),
            Err(ProtocolError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn snapshot_hash_mismatch_is_rejected() {
        let limits = Limits::default();
        let mut bytes = sample_snapshot().encode(&limits).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(matches!(
            Packet::decode(&bytes, &limits),
            Err(ProtocolError::HashMismatch { .. })
        ));
    }

    #[test]
    fn invalid_limits_are_rejected() {
        for limits in [
            Limits {
                max_input_bytes: 0,
                ..Limits::default()
            },
            Limits {
                max_snapshot_bytes: 0,
                ..Limits::default()
            },
            Limits {
                max_ack_history: 0,
                ..Limits::default()
            },
            Limits {
                max_snapshot_bytes: MAX_PAYLOAD_BYTES.saturating_add(1),
                ..Limits::default()
            },
        ] {
            assert!(matches!(
                limits.validate(),
                Err(ProtocolError::InvalidLimits(_))
            ));
        }
    }
}
