use fate_sim::StateWriter;

use crate::sim::{Command, Duel, LIMIT, SNAPSHOT_MAX};

const MAGIC: [u8; 4] = *b"DREP";
const VERSION: u16 = 5;
const HEADER: usize = 4 + 2 + 2 + 4;
const INPUT_BYTES: usize = 5;
const FRAME_BYTES: usize = INPUT_BYTES + 8;
pub const MAX_BYTES: usize = HEADER + SNAPSHOT_MAX + LIMIT as usize * FRAME_BYTES;

struct Frame {
    command: Command,
    hash: u64,
}

/// A project-local recording. Restores start a new branch rooted at the
/// restored snapshot rather than mixing different timelines in one file.
pub struct Replay {
    initial: Vec<u8>,
    frames: Vec<Frame>,
}

impl Replay {
    pub fn new(start: &Duel) -> Self {
        Self {
            initial: start.snapshot(),
            frames: Vec::new(),
        }
    }

    pub fn record(&mut self, command: Command, state: &Duel) {
        self.frames.push(Frame {
            command,
            hash: state.hash(),
        });
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes =
            Vec::with_capacity(HEADER + self.initial.len() + self.frames.len() * FRAME_BYTES);
        bytes.write_bytes(&MAGIC);
        bytes.write_u16(VERSION);
        bytes.write_u16(self.initial.len() as u16);
        bytes.write_u32(self.frames.len() as u32);
        bytes.write_bytes(&self.initial);
        for frame in &self.frames {
            let command = frame.command;
            let buttons = u8::from(command.guard)
                | (u8::from(command.attack) << 1)
                | (u8::from(command.boost) << 2)
                | (u8::from(command.dash) << 3);
            bytes.write_bytes(&[
                command.x as u8,
                command.z as u8,
                command.aim_x as u8,
                command.aim_z as u8,
                buttons,
            ]);
            bytes.write_u64(frame.hash);
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let (header, payload) = bytes
            .split_first_chunk::<HEADER>()
            .ok_or("truncated replay header")?;
        if header[..4] != MAGIC || u16::from_le_bytes([header[4], header[5]]) != VERSION {
            return Err("unsupported replay format");
        }
        let snapshot_len = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let frame_count = u32::from_le_bytes(header[8..12].try_into().expect("fixed header"));
        if snapshot_len > SNAPSHOT_MAX || frame_count > LIMIT {
            return Err("replay exceeds size limits");
        }
        let frame_count = frame_count as usize;
        if payload.len() != snapshot_len + frame_count * FRAME_BYTES {
            return Err("truncated or trailing replay data");
        }
        let (initial, packed) = payload.split_at(snapshot_len);
        let start = Duel::restore(initial)?;
        if frame_count > (LIMIT - start.tick) as usize {
            return Err("replay extends past the round limit");
        }

        let mut frames = Vec::with_capacity(frame_count);
        for frame in packed.as_chunks::<FRAME_BYTES>().0 {
            let [x, z, aim_x, aim_z] = frame[..INPUT_BYTES - 1].try_into().expect("fixed axes");
            let buttons = frame[INPUT_BYTES - 1];
            if [x, z, aim_x, aim_z]
                .into_iter()
                .any(|axis| axis == i8::MIN as u8)
                || buttons & !0b1111 != 0
            {
                return Err("invalid replay command");
            }
            frames.push(Frame {
                command: Command {
                    x: x as i8,
                    z: z as i8,
                    aim_x: aim_x as i8,
                    aim_z: aim_z as i8,
                    guard: buttons & 1 != 0,
                    attack: buttons & 2 != 0,
                    boost: buttons & 4 != 0,
                    dash: buttons & 8 != 0,
                },
                hash: u64::from_le_bytes(frame[INPUT_BYTES..].try_into().expect("fixed hash")),
            });
        }
        Ok(Self {
            initial: initial.to_vec(),
            frames,
        })
    }

    pub fn verify(&self) -> Result<Duel, String> {
        let mut duel = Duel::restore(&self.initial)?;
        for frame in &self.frames {
            let tick = duel.tick + 1;
            duel.step(frame.command);
            if duel.tick != tick {
                return Err(format!(
                    "recording continues after the round ended at tick {}",
                    duel.tick
                ));
            }
            let actual = duel.hash();
            if actual != frame.hash {
                return Err(format!(
                    "replay diverged at tick {tick}: expected {:016x}, got {actual:016x}",
                    frame.hash
                ));
            }
        }
        Ok(duel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_from_a_restored_snapshot_and_reports_the_first_divergence() {
        let mut duel = Duel::new();
        for tick in 1..=crate::CHECKPOINT {
            duel.step(crate::command_at(tick));
        }
        assert!(!duel.bolts.is_empty());
        let mut replay = Replay::new(&duel);
        for tick in (crate::CHECKPOINT + 1)..=crate::SCRIPT_END {
            let command = crate::command_at(tick);
            duel.step(command);
            replay.record(command, &duel);
        }
        let mut encoded = replay.encode();
        let decoded = Replay::decode(&encoded).unwrap();
        assert_eq!(decoded.verify().unwrap().snapshot(), duel.snapshot());
        assert_eq!(decoded.encode(), encoded);

        let hash_start = HEADER + replay.initial.len() + INPUT_BYTES;
        encoded[hash_start] ^= 1;
        let error = Replay::decode(&encoded).unwrap().verify().unwrap_err();
        let first = crate::CHECKPOINT + 1;
        assert!(error.contains(&format!("tick {first}")), "{error}");
    }

    #[test]
    fn rejects_truncation_bad_commands_and_out_of_range_counts() {
        let mut duel = Duel::new();
        let mut replay = Replay::new(&duel);
        let command = Command::default();
        duel.step(command);
        replay.record(command, &duel);
        let bytes = replay.encode();
        for end in 0..bytes.len() {
            assert!(Replay::decode(&bytes[..end]).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(Replay::decode(&extra).is_err());
        let mut too_many = bytes.clone();
        too_many[8..12].copy_from_slice(&(LIMIT + 1).to_le_bytes());
        assert!(Replay::decode(&too_many).is_err());
        let first_command = HEADER + replay.initial.len();
        let mut invalid = bytes.clone();
        invalid[first_command] = i8::MIN as u8;
        assert!(Replay::decode(&invalid).is_err());
        let mut invalid = bytes;
        invalid[first_command + INPUT_BYTES - 1] = 0b1_0000;
        assert!(Replay::decode(&invalid).is_err());
    }
}
