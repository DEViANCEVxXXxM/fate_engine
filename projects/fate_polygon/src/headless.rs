//! Fixed-tick CPU benchmark. Does not initialize the native app or renderer.

use std::io::{self, Write};
use std::time::Instant;

use crate::bench_io::{Percentiles, open_csv};
use crate::cli::{Bench, Case};
use crate::lab::{Lab, Work};
use crate::settings::Settings;

#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    pairs: u64,
    contacts: u64,
    decisions: u64,
    neighbors: u64,
}

struct ResultRow {
    tick: Percentiles,
    physics: Option<Percentiles>,
    ai: Option<Percentiles>,
    work: Totals,
    digest: u64,
}

fn measure(s: Settings, warmup: usize, ticks: usize) -> ResultRow {
    let mut lab = Lab::default();
    lab.sync_counts(s);
    for _ in 0..warmup {
        lab.step(s, &mut Work::default());
    }
    let mut all = Vec::with_capacity(ticks);
    let mut physics = Vec::with_capacity(if s.physics { ticks } else { 0 });
    let mut ai = Vec::with_capacity(if s.ai { ticks } else { 0 });
    let mut totals = Totals::default();
    for _ in 0..ticks {
        let mut work = Work::default();
        let start = Instant::now();
        lab.step(s, &mut work);
        all.push(start.elapsed());
        if s.physics {
            physics.push(work.physics);
        }
        if s.ai {
            ai.push(work.ai);
        }
        totals.pairs += work.pairs;
        totals.contacts += work.contacts;
        totals.decisions += work.decisions;
        totals.neighbors += work.neighbors;
    }
    ResultRow {
        tick: Percentiles::from_samples(all).expect("nonempty measurement"),
        physics: Percentiles::from_samples(physics),
        ai: Percentiles::from_samples(ai),
        work: totals,
        digest: lab.state_hash(),
    }
}

pub fn run(bench: Bench) -> io::Result<()> {
    let mut csv = open_csv(bench.csv.as_deref())?;
    let mut output = io::stdout().lock();
    if let Some(csv) = &mut csv {
        csv_header(csv)?;
    }
    writeln!(
        output,
        "fate_polygon | CPU lab only (no window, GPU or audio)"
    )?;
    writeln!(
        output,
        "{} preset | {} case(s) | {} warmup + {} measured ticks per case",
        bench.preset.name(),
        bench.cases.len(),
        bench.warmup,
        bench.ticks
    )?;
    writeln!(
        output,
        "Durations: p50/p95 ms per tick. Phase timings are included in tick time."
    )?;
    for (index, case) in bench.cases.iter().enumerate() {
        let row = measure(case.settings, bench.warmup, bench.ticks);
        print_row(&mut output, index + 1, case, &row)?;
        if let Some(csv) = &mut csv {
            csv_row(csv, &bench, index + 1, case, &row)?;
        }
    }
    if let Some(mut writer) = csv {
        writer.flush()?;
        writeln!(output, "CSV: {}", bench.csv.expect("CSV path").display())?;
    }
    Ok(())
}

fn print_row(out: &mut impl Write, index: usize, case: &Case, row: &ResultRow) -> io::Result<()> {
    let s = case.settings;
    writeln!(out, "\nCase {index}")?;
    if let Some((name, value)) = case.swept {
        writeln!(out, "  sweep: {name}={value}")?;
    }
    writeln!(
        out,
        "  seed {} | {} Hz | physics {} | AI {}",
        s.seed,
        s.tick_hz,
        if s.physics { "on" } else { "off" },
        if s.ai { "on" } else { "off" }
    )?;
    writeln!(
        out,
        "  tick    {:>9.4} / {:<9.4}",
        row.tick.p50, row.tick.p95
    )?;
    if let Some(p) = row.physics {
        writeln!(
            out,
            "  physics {:>9.4} / {:<9.4}   {} bodies, {} search, {} pair tests, {} contacts",
            p.p50,
            p.p95,
            s.bodies,
            if s.broad_phase == 1 {
                "grid"
            } else {
                "all-pairs"
            },
            row.work.pairs,
            row.work.contacts
        )?;
    }
    if let Some(p) = row.ai {
        writeln!(
            out,
            "  AI      {:>9.4} / {:<9.4}   {} agents, {} decisions, {} neighbor tests",
            p.p50, p.p95, s.agents, row.work.decisions, row.work.neighbors
        )?;
    }
    writeln!(out, "  final state {:016x}", row.digest)
}

fn csv_header(out: &mut impl Write) -> io::Result<()> {
    write!(
        out,
        "case,preset,sweep_setting,sweep_value,warmup_ticks,measured_ticks,physics,ai"
    )?;
    for spec in Settings::NUMBERS.iter().filter(|spec| spec.cpu) {
        write!(out, ",{}", spec.name)?;
    }
    writeln!(
        out,
        ",tick_p50_ms,tick_p95_ms,physics_p50_ms,physics_p95_ms,ai_p50_ms,ai_p95_ms,pair_tests,contacts,decisions,neighbor_tests,state_hash"
    )
}

fn csv_row(
    out: &mut impl Write,
    bench: &Bench,
    index: usize,
    case: &Case,
    row: &ResultRow,
) -> io::Result<()> {
    let s = case.settings;
    let (name, value) = case.swept.map_or(("", String::new()), |(name, value)| {
        (name, value.to_string())
    });
    write!(
        out,
        "{index},{},{name},{value},{},{},{},{}",
        bench.preset.name(),
        bench.warmup,
        bench.ticks,
        s.physics,
        s.ai
    )?;
    for spec in Settings::NUMBERS.iter().filter(|spec| spec.cpu) {
        write!(
            out,
            ",{}",
            s.number_value(spec.name).expect("known setting")
        )?;
    }
    for phase in [Some(row.tick), row.physics, row.ai] {
        if let Some(p) = phase {
            write!(out, ",{:.6},{:.6}", p.p50, p.p95)?;
        } else {
            write!(out, ",,")?;
        }
    }
    writeln!(
        out,
        ",{},{},{},{},{:016x}",
        row.work.pairs, row.work.contacts, row.work.decisions, row.work.neighbors, row.digest
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Preset;

    #[test]
    fn deterministic_runs_exclude_warmup_from_work_but_not_state() {
        let s = Settings {
            bodies: 10,
            passes: 2,
            agents: 12,
            samples: 3,
            think_every: 1,
            ..Settings::default()
        };
        let first = measure(s, 3, 8);
        let second = measure(s, 3, 8);
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.work, second.work);
        assert_eq!(first.work.pairs, 8 * 2 * 45);
        assert_eq!(first.work.neighbors, 8 * 12 * 3);
        assert_eq!(first.digest, measure(s, 0, 11).digest);
        assert_ne!(
            first.digest,
            measure(
                Settings {
                    seed: s.seed + 1,
                    ..s
                },
                3,
                8
            )
            .digest
        );
    }

    #[test]
    fn csv_is_rectangular_and_disabled_phases_are_blank() {
        let mut data = Vec::new();
        let bench = Bench {
            preset: Preset::Physics,
            cases: vec![Case {
                settings: Settings {
                    bodies: 8,
                    ..Settings::preset(Preset::Physics)
                },
                swept: Some(("bodies", 8)),
            }],
            warmup: 0,
            ticks: 4,
            csv: None,
        };
        let case = &bench.cases[0];
        let result = measure(case.settings, bench.warmup, bench.ticks);
        csv_header(&mut data).unwrap();
        csv_row(&mut data, &bench, 1, case, &result).unwrap();
        let data = String::from_utf8(data).unwrap();
        let mut lines = data.lines().map(|line| line.split(',').collect::<Vec<_>>());
        let header = lines.next().unwrap();
        let row = lines.next().unwrap();
        assert_eq!(header.len(), row.len());
        let field = |name| row[header.iter().position(|&key| key == name).unwrap()];
        assert_eq!(field("bodies"), "8");
        assert_eq!(field("pair_tests"), "224");
        assert_eq!(field("ai_p50_ms"), "");
        assert_eq!(field("ai_p95_ms"), "");
        assert_eq!(field("state_hash"), format!("{:016x}", result.digest));
    }
}
