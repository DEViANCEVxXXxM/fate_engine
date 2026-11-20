use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
}

impl Percentiles {
    pub fn from_samples(mut samples: Vec<Duration>) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        samples.sort_unstable();
        let percentile =
            |p: usize| samples[(samples.len() * p).div_ceil(100) - 1].as_secs_f64() * 1e3;
        Some(Self {
            p50: percentile(50),
            p95: percentile(95),
        })
    }
}

pub fn open_csv(path: Option<&Path>) -> io::Result<Option<BufWriter<File>>> {
    path.map(|path| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(BufWriter::new)
            .map_err(|e| {
                io::Error::new(e.kind(), format!("cannot create '{}': {e}", path.display()))
            })
    })
    .transpose()
}
