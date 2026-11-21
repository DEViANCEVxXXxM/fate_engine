//! Headless file tools for showcase-owned replays.

use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use crate::demo_replay::{ReplayError, ReplayTape};
use crate::showcase::ShowcaseSim;

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

pub fn run(record: bool, path: &Path) -> i32 {
    let result = if record {
        record_sample(path, &mut io::stdout().lock())
    } else {
        inspect(path, &mut io::stdout().lock())
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            if matches!(
                error.downcast_ref::<ReplayError>(),
                Some(ReplayError::Diverged { .. })
            ) {
                1
            } else {
                2
            }
        }
    }
}

fn record_sample(path: &Path, out: &mut impl Write) -> Result<(), Box<dyn Error>> {
    let tape = ShowcaseSim::new().scripted_replay()?;
    let bytes = tape.to_bytes()?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    let (start, end) = tape.tick_range();
    writeln!(
        out,
        "Recorded {} inputs: ticks {start}..={end}",
        tape.tick_count()
    )?;
    Ok(())
}

fn inspect(path: &Path, out: &mut impl Write) -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "replay exceeds 128 MiB").into());
    }
    let tape = ReplayTape::from_bytes(&bytes)?;
    let (start, end) = tape.tick_range();
    writeln!(
        out,
        "Replay: ticks {start}..={end} ({} inputs)",
        tape.tick_count()
    )?;
    let state = tape.playback()?;
    writeln!(out, "Verified; final checksum {:016x}", state.state_hash())?;
    Ok(())
}
