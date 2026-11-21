//! Fixed-tick CPU benchmark. Does not initialize the native app or renderer.

use std::io::{self, Write};
use std::time::Instant;

use crate::bench_io::{Percentiles, open_csv};
use crate::bench_schema::Kind;
use crate::cli::{Bench, Case};
use crate::lab::{Lab, Work};
use crate::navigation::RouteWork;
use crate::settings::Settings;

#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    pairs: u64,
    contacts: u64,
    decisions: u64,
    neighbors: u64,
    route: RouteWork,
    ecs: EcsTotals,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct EcsTotals {
    spawned: u64,
    despawned: u64,
    updates: u64,
    snapshots: u64,
    bytes: u64,
}

struct ResultRow {
    tick: Percentiles,
    physics: Option<Percentiles>,
    ai: Option<Percentiles>,
    ecs: Option<Percentiles>,
    snapshot: Option<Percentiles>,
    snapshot_phases: Option<[Percentiles; 3]>,
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
    let mut ecs = Vec::with_capacity(if s.ecs { ticks } else { 0 });
    let mut snapshots = Vec::new();
    let mut phases = [Vec::new(), Vec::new(), Vec::new()];
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
        if s.ecs {
            ecs.push(work.ecs.update_time);
            if work.ecs.snapshots != 0 {
                snapshots.push(work.ecs.snapshot_time);
                for (samples, time) in phases.iter_mut().zip([
                    work.ecs.encode_time,
                    work.ecs.decode_time,
                    work.ecs.verify_time,
                ]) {
                    samples.push(time);
                }
            }
        }
        totals.pairs += work.pairs;
        totals.contacts += work.contacts;
        totals.decisions += work.decisions;
        totals.neighbors += work.neighbors;
        totals.route.searches += work.route.searches;
        totals.route.visits += work.route.visits;
        totals.route.moves += work.route.moves;
        totals.ecs.spawned += work.ecs.spawned;
        totals.ecs.despawned += work.ecs.despawned;
        totals.ecs.updates += work.ecs.updates;
        totals.ecs.snapshots += work.ecs.snapshots;
        totals.ecs.bytes += work.ecs.bytes;
    }
    let snapshot_phases = (!snapshots.is_empty()).then(|| {
        phases.map(|samples| Percentiles::from_samples(samples).expect("sampled every phase"))
    });
    ResultRow {
        tick: Percentiles::from_samples(all).expect("nonempty measurement"),
        physics: Percentiles::from_samples(physics),
        ai: Percentiles::from_samples(ai),
        ecs: Percentiles::from_samples(ecs),
        snapshot: Percentiles::from_samples(snapshots),
        snapshot_phases,
        work: totals,
        digest: lab.state_hash(),
    }
}

pub fn run(bench: Bench) -> io::Result<()> {
    let mut csv = open_csv(bench.csv.as_deref())?;
    let mut output = io::stdout().lock();
    if let Some(csv) = &mut csv {
        csv_header(csv, cpu_kind(bench.cases[0].settings))?;
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
        if s.nav_mode == 0 {
            writeln!(
                out,
                "  AI      {:>9.4} / {:<9.4}   {} agents, {} decisions, {} neighbor tests",
                p.p50, p.p95, s.agents, row.work.decisions, row.work.neighbors
            )?;
        } else {
            writeln!(
                out,
                "  AI      {:>9.4} / {:<9.4}   {} agents, {} navigation, {} searches, {} cell visits, {} moves",
                p.p50,
                p.p95,
                s.agents,
                if s.nav_mode == 1 {
                    "per-agent"
                } else {
                    "shared"
                },
                row.work.route.searches,
                row.work.route.visits,
                row.work.route.moves
            )?;
        }
    }
    if let Some(p) = row.ecs {
        let work = &row.work.ecs;
        writeln!(
            out,
            "  ECS     {:>9.4} / {:<9.4}   {} updates, {} spawns, {} despawns",
            p.p50, p.p95, work.updates, work.spawned, work.despawned
        )?;
        if let Some(p) = row.snapshot {
            writeln!(
                out,
                "  restore {:>9.4} / {:<9.4}   {} snapshots, {} encoded bytes (ms per snapshot)",
                p.p50, p.p95, work.snapshots, work.bytes
            )?;
            for (name, phase) in ["encode", "decode", "verify"]
                .into_iter()
                .zip(row.snapshot_phases.expect("snapshot phase samples"))
            {
                writeln!(out, "    {name:<7} {:>9.4} / {:<9.4}", phase.p50, phase.p95)?;
            }
        } else {
            writeln!(out, "  restore no snapshots in the measured interval")?;
        }
    }
    writeln!(out, "  final state {:016x}", row.digest)
}

fn cpu_kind(settings: Settings) -> Kind {
    if settings.ecs { Kind::Ecs } else { Kind::Cpu }
}

fn csv_header(out: &mut impl Write, kind: Kind) -> io::Result<()> {
    writeln!(out, "{}", kind.columns().join(","))
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
    for name in cpu_kind(s).cpu_numbers() {
        write!(out, ",{}", s.number_value(name).expect("known setting"))?;
    }
    if s.ecs {
        write!(out, ",true")?;
    }
    let timings = [Some(row.tick), row.physics, row.ai, row.ecs, row.snapshot];
    for phase in &timings[..if s.ecs { 5 } else { 3 }] {
        if let Some(p) = phase {
            write!(out, ",{:.6},{:.6}", p.p50, p.p95)?;
        } else {
            write!(out, ",,")?;
        }
    }
    write!(
        out,
        ",{},{},{},{},{},{},{},{:016x}",
        row.work.pairs,
        row.work.contacts,
        row.work.decisions,
        row.work.neighbors,
        row.work.route.searches,
        row.work.route.visits,
        row.work.route.moves,
        row.digest
    )?;
    if s.ecs {
        let work = &row.work.ecs;
        write!(
            out,
            ",{},{},{},{},{}",
            work.spawned, work.despawned, work.updates, work.snapshots, work.bytes
        )?;
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Preset;

    #[test]
    fn portable_cpu_baseline() {
        // Keep in sync with the reference runs in docs/roadmap.md.
        // Timing fields are intentionally not part of this contract.
        let run = |settings: Settings| {
            measure(
                Settings {
                    seed: 71,
                    ..settings
                },
                16,
                64,
            )
        };
        let physics = Settings::preset(Preset::Physics);
        for (broad_phase, pairs) in [(0, 4_177_920), (1, 208_700)] {
            let row = run(Settings {
                broad_phase,
                ..physics
            });
            assert_eq!(
                row.digest, 0x1b45_65dc_b127_5112,
                "broad_phase={broad_phase}"
            );
            assert_eq!(
                (row.work.pairs, row.work.contacts),
                (pairs, 243),
                "broad_phase={broad_phase}"
            );
        }

        let navigation = Settings::preset(Preset::Navigation);
        for (nav_mode, searches, visits) in [(1, 1_354, 401_256), (2, 4, 2_368)] {
            let row = run(Settings {
                nav_mode,
                nav_goal_ticks: 16,
                ..navigation
            });
            assert_eq!(row.digest, 0x349b_9e70_4332_dc5b, "nav_mode={nav_mode}");
            assert_eq!(
                row.work.route,
                RouteWork {
                    searches,
                    visits,
                    moves: 1_354
                },
                "nav_mode={nav_mode}",
            );
        }

        let ecs = Settings::preset(Preset::Ecs);
        for (snapshot_every, snapshots, bytes) in [(0, 0, 0), (16, 4, 442_488)] {
            let row = run(Settings {
                snapshot_every,
                ..ecs
            });
            assert_eq!(
                row.digest, 0x90fa_f31c_c4d5_c1f3,
                "snapshot_every={snapshot_every}"
            );
            assert_eq!(
                row.work.ecs,
                EcsTotals {
                    spawned: 8_192,
                    despawned: 8_192,
                    updates: 131_072,
                    snapshots,
                    bytes
                },
                "snapshot_every={snapshot_every}",
            );
        }
    }

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
    fn ecs_counts_and_snapshot_samples_exclude_warmup() {
        let s = Settings {
            entities: 32,
            churn: 4,
            snapshot_every: 4,
            ..Settings::preset(Preset::Ecs)
        };
        let row = measure(s, 4, 9);
        let ecs = &row.work.ecs;
        assert_eq!(
            (ecs.spawned, ecs.despawned, ecs.updates, ecs.snapshots),
            (36, 36, 288, 2)
        );
        assert!(row.ecs.is_some() && row.snapshot.is_some());
        assert!(row.snapshot_phases.is_some());
        let no_restore = measure(
            Settings {
                snapshot_every: 0,
                ..s
            },
            4,
            9,
        );
        assert!(no_restore.snapshot.is_none());
        assert!(no_restore.snapshot_phases.is_none());
        assert_eq!(no_restore.digest, row.digest);
        assert!(!Kind::Cpu.columns().contains(&"entities"));
        assert!(Kind::Ecs.columns().contains(&"snapshot_bytes"));
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
        csv_header(&mut data, Kind::Cpu).unwrap();
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
