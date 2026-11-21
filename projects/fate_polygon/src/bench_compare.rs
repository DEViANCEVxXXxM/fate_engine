//! Compare Polygon benchmark artifacts without opening a window or device.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::bench_schema::Kind;

const DEFAULT_PERCENT: f64 = 10.0;
const DEFAULT_FLOOR_MS: f64 = 0.005;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub struct Options {
    baseline: PathBuf,
    current: PathBuf,
    thresholds: Thresholds,
}

#[derive(Clone, Copy)]
struct Thresholds {
    percent: f64,
    floor_ms: f64,
}

pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut path = || {
        args.next()
            .filter(|path| !path.is_empty() && !path.starts_with("--"))
            .map(PathBuf::from)
            .ok_or_else(|| "--compare expects BASELINE.csv CURRENT.csv".to_owned())
    };
    let baseline = path()?;
    let current = path()?;
    let mut percent = None;
    let mut floor_ms = None;
    while let Some(arg) = args.next() {
        let setting = match arg.as_str() {
            "--max-regression-pct" if percent.is_none() => &mut percent,
            "--noise-floor-ms" if floor_ms.is_none() => &mut floor_ms,
            _ => return Err(format!("unknown or repeated comparison option '{arg}'")),
        };
        *setting = Some(
            args.next()
                .and_then(|arg| arg.parse::<f64>().ok())
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| format!("{arg} expects a finite, non-negative number"))?,
        );
    }
    Ok(Options {
        baseline,
        current,
        thresholds: Thresholds {
            percent: percent.unwrap_or(DEFAULT_PERCENT),
            floor_ms: floor_ms.unwrap_or(DEFAULT_FLOOR_MS),
        },
    })
}

pub fn run(options: Options) -> Result<bool, Box<dyn std::error::Error>> {
    let baseline = Dataset::read(&options.baseline)?;
    let current = Dataset::read(&options.current)?;
    Ok(compare(
        &baseline,
        &current,
        options.thresholds,
        &mut io::stdout().lock(),
    )?)
}

struct Dataset {
    kind: Kind,
    rows: BTreeMap<Vec<String>, Row>,
}

struct Row {
    case: u64,
    times: [Option<f64>; 10],
    work: Vec<String>,
}

impl Dataset {
    fn read(path: &Path) -> Result<Self, String> {
        let read = || -> Result<Self, String> {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let mut text = String::new();
            file.take(MAX_FILE_BYTES + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            if text.len() as u64 > MAX_FILE_BYTES {
                return Err("CSV exceeds 2 MiB".to_owned());
            }
            Self::parse(&text)
        };
        read().map_err(|e| format!("{}: {e}", path.display()))
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut records = csv_records(text.trim_start_matches('\u{feff}'))?.into_iter();
        let header = records.next().ok_or_else(|| "empty CSV".to_owned())?;
        let columns: BTreeMap<_, _> = header
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();
        let kind = [Kind::Cpu, Kind::Ecs, Kind::Offscreen]
            .into_iter()
            .find(|kind| {
                let expected = kind.columns();
                header.len() == expected.len()
                    && columns.len() == expected.len()
                    && expected.into_iter().all(|name| columns.contains_key(name))
            })
            .ok_or_else(|| {
                "unsupported CSV schema; use a current Polygon benchmark export".to_owned()
            })?;
        let mut rows = BTreeMap::new();
        let inputs = kind.inputs();
        let time_columns = kind.timings();
        let work_columns = kind.work();
        for (i, record) in records.enumerate() {
            let parse = || -> Result<_, String> {
                if header.len() != record.len() {
                    return Err("wrong column count".to_owned());
                }
                let field = |name: &str| record[columns[name]].as_str();
                let integer = |name: &str| {
                    field(name)
                        .parse::<u64>()
                        .map_err(|_| format!("invalid integer in '{name}'"))
                };
                let flag = |name: &str| {
                    field(name)
                        .parse::<bool>()
                        .map_err(|_| format!("invalid boolean in '{name}'"))
                };
                let mut key = Vec::new();
                for &name in &inputs {
                    key.push(match name {
                        "adapter" | "backend" | "driver" => field(name).to_owned(),
                        "physics" | "ai" | "ecs" | "cull" => flag(name)?.to_string(),
                        _ => integer(name)?.to_string(),
                    });
                }
                let enabled = match kind {
                    Kind::Cpu | Kind::Ecs => {
                        if integer("measured_ticks")? == 0 {
                            return Err("CPU run has no measured ticks".to_owned());
                        }
                        let ecs = kind == Kind::Ecs;
                        if ecs && !flag("ecs")? {
                            return Err("ECS export must have its ECS workload enabled".to_owned());
                        }
                        [
                            true,
                            flag("physics")?,
                            flag("ai")?,
                            ecs,
                            ecs && integer("snapshots")? != 0,
                        ]
                    }
                    Kind::Offscreen => {
                        let frames = integer("measured_frames")?;
                        let samples = integer("gpu_samples")?;
                        if frames == 0 || (samples != 0 && samples != frames) {
                            return Err("offscreen run needs either zero or one GPU timestamp per measured frame".to_owned());
                        }
                        [true, true, samples != 0, false, false]
                    }
                };
                let mut times = [None; 10];
                for (i, name) in time_columns.iter().enumerate() {
                    if enabled[i / 2] {
                        times[i] = Some(
                            field(name)
                                .parse::<f64>()
                                .ok()
                                .filter(|n| n.is_finite() && *n >= 0.0)
                                .ok_or_else(|| format!("invalid timing in '{name}'"))?,
                        );
                    } else if !field(name).is_empty() {
                        return Err(format!("expected blank timing in '{name}'"));
                    }
                }
                for i in (0..time_columns.len()).step_by(2) {
                    if let (Some(p50), Some(p95)) = (times[i], times[i + 1])
                        && p50 > p95
                    {
                        return Err(format!(
                            "'{}' exceeds '{}'",
                            time_columns[i],
                            time_columns[i + 1]
                        ));
                    }
                }
                let work = work_columns
                    .iter()
                    .map(|&name| {
                        if name != "state_hash" {
                            return Ok(integer(name)?.to_string());
                        }
                        let hex = field(name);
                        if hex.len() != 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                            return Err("invalid state hash".to_owned());
                        }
                        Ok(hex.to_ascii_lowercase())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((
                    key,
                    Row {
                        case: integer("case")?,
                        times,
                        work,
                    },
                ))
            };
            let (key, row) = parse().map_err(|error| format!("record {}: {error}", i + 2))?;
            if rows.insert(key, row).is_some() {
                return Err(format!("record {}: duplicate effective settings", i + 2));
            }
        }
        if rows.is_empty() {
            return Err("CSV has no benchmark cases".to_owned());
        }
        Ok(Self { kind, rows })
    }
}

fn compare(
    baseline: &Dataset,
    current: &Dataset,
    thresholds: Thresholds,
    out: &mut impl Write,
) -> io::Result<bool> {
    let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidData, error);
    if baseline.kind != current.kind {
        return Err(invalid(
            "benchmark kinds are different; compare matching CPU, ECS or offscreen exports"
                .to_owned(),
        ));
    }
    for (base, other, side) in [
        (baseline, current, "baseline"),
        (current, baseline, "current"),
    ] {
        for (key, row) in &base.rows {
            if !other.rows.contains_key(key) {
                return Err(invalid(format!(
                    "{side} case {} has no matching settings (including adapter/driver for offscreen runs)",
                    row.case
                )));
            }
        }
    }
    for (key, old) in &baseline.rows {
        let new = &current.rows[key];
        if old.work == new.work
            && old
                .times
                .iter()
                .zip(new.times)
                .any(|(a, b)| a.is_some() != b.is_some())
        {
            return Err(invalid(format!(
                "baseline case {} and current case {} have different timing availability",
                old.case, new.case
            )));
        }
    }

    let (timings, work) = (baseline.kind.timings(), baseline.kind.work());
    writeln!(
        out,
        "fate_polygon | {:?} comparison | {} matched cases",
        baseline.kind,
        baseline.rows.len()
    )?;
    writeln!(
        out,
        "Regressions must exceed both {:.2}% and {:.6} ms.",
        thresholds.percent, thresholds.floor_ms
    )?;
    let mut failures = 0;
    for (key, old) in &baseline.rows {
        let new = &current.rows[key];
        writeln!(
            out,
            "\nBaseline case {} -> current case {}",
            old.case, new.case
        )?;
        if old.work != new.work {
            for ((label, before), after) in work.iter().zip(&old.work).zip(&new.work) {
                if before != after {
                    writeln!(out, "  WORK CHANGED {label}: {before} -> {after}")?;
                }
            }
            writeln!(out, "  Skipped timing comparison because the work changed.")?;
            failures += 1;
            continue;
        }
        if baseline.kind == Kind::Offscreen && old.times[4].is_none() {
            writeln!(
                out,
                "  GPU scene unavailable in both runs; comparing CPU only."
            )?;
        }
        for (&label, (before, after)) in timings.iter().zip(old.times.into_iter().zip(new.times)) {
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            let delta = after - before;
            let regression =
                delta > thresholds.floor_ms && delta > before * (thresholds.percent / 100.0);
            let status = if regression { "REGRESSION" } else { "OK" };
            failures += usize::from(regression);
            writeln!(
                out,
                "  {label:<22} {before:.6} -> {after:.6} ms ({delta:+.6})  {status}"
            )?;
        }
    }
    writeln!(
        out,
        "\n{}",
        if failures == 0 {
            "PASS".to_owned()
        } else {
            format!("FAIL: {failures} work or timing check(s)")
        }
    )?;
    Ok(failures == 0)
}

/// Parse the CSV exported by Polygon, including quoted adapter/driver metadata.
fn csv_records(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut chars = text.chars().peekable();
    let mut records = Vec::new();
    let mut record = Vec::new();
    while chars.peek().is_some() {
        let mut field = String::new();
        if chars.next_if_eq(&'"').is_some() {
            loop {
                match chars.next() {
                    Some('"') if chars.next_if_eq(&'"').is_some() => field.push('"'),
                    Some('"') => break,
                    Some(ch) => field.push(ch),
                    None => return Err("unclosed CSV quote".to_owned()),
                }
            }
        } else {
            while let Some(ch) = chars.next_if(|ch| !matches!(ch, ',' | '\r' | '\n' | '"')) {
                field.push(ch);
            }
        }
        record.push(field);
        match chars.next() {
            Some(',') if chars.peek().is_some() => continue,
            Some(',') => record.push(String::new()),
            Some('\r') => {
                chars.next_if_eq(&'\n');
            }
            Some('\n') | None => {}
            Some(_) => return Err("unexpected character after CSV field".to_owned()),
        }
        records.push(std::mem::take(&mut record));
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    const THRESHOLDS: Thresholds = Thresholds {
        percent: 10.0,
        floor_ms: 0.005,
    };

    fn cpu_row(overrides: &[(&str, &str)]) -> String {
        Kind::Cpu
            .columns()
            .into_iter()
            .map(|name| {
                if let Some((_, value)) = overrides.iter().find(|(key, _)| *key == name) {
                    return (*value).to_owned();
                }
                if let Some(value) = Settings::default().number_value(name) {
                    return value.to_string();
                }
                match name {
                    "case" => "1",
                    "preset" => "physics",
                    "sweep_setting" | "sweep_value" | "ai_p50_ms" | "ai_p95_ms" => "",
                    "warmup_ticks" => "8",
                    "measured_ticks" => "100",
                    "physics" => "true",
                    "ai" => "false",
                    "tick_p50_ms" | "physics_p50_ms" => "0.001",
                    "tick_p95_ms" | "physics_p95_ms" => "0.100",
                    "state_hash" => "0123456789abcdef",
                    _ => "0",
                }
                .to_owned()
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn gpu_row(adapter: &str, samples: &str) -> String {
        Kind::Offscreen
            .columns()
            .into_iter()
            .map(|name| match name {
                "case" => "1",
                "preset" => "render",
                "sweep_setting" | "sweep_value" => "",
                "adapter" => adapter,
                "backend" => "Vulkan",
                "driver" => "test",
                "width" | "height" => "540",
                "measured_frames" => "10",
                "detail" | "batch" => "8",
                "cull" => "false",
                "gpu_samples" => samples,
                "gpu_scene_p50_ms" | "gpu_scene_p95_ms" if samples == "0" => "",
                _ => "0",
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn data(kind: Kind, rows: &[String]) -> Result<Dataset, String> {
        Dataset::parse(&format!(
            "{}\r\n{}\r\n",
            kind.columns().join(","),
            rows.join("\r\n")
        ))
    }

    #[test]
    fn matches_configuration_not_row_number_and_applies_both_thresholds() {
        let baseline = data(
            Kind::Cpu,
            &[cpu_row(&[]), cpu_row(&[("case", "2"), ("bodies", "123")])],
        )
        .unwrap();
        let current = data(
            Kind::Cpu,
            &[
                cpu_row(&[("case", "1"), ("bodies", "123")]),
                cpu_row(&[
                    ("case", "2"),
                    ("preset", "mixed"),
                    ("tick_p50_ms", "0.004"),
                    ("physics_p95_ms", "0.109"),
                ]),
            ],
        )
        .unwrap();
        let mut out = Vec::new();
        assert!(compare(&baseline, &current, THRESHOLDS, &mut out).unwrap());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Baseline case 1 -> current case 2"));
        assert!(text.contains("PASS"));

        let baseline = data(Kind::Cpu, &[cpu_row(&[])]).unwrap();
        let slower = data(Kind::Cpu, &[cpu_row(&[("tick_p95_ms", "0.121")])]).unwrap();
        let mut out = Vec::new();
        assert!(!compare(&baseline, &slower, THRESHOLDS, &mut out).unwrap());
        assert!(String::from_utf8(out).unwrap().contains("REGRESSION"));
    }

    #[test]
    fn changed_work_is_a_failure_and_not_compared_as_a_timing_regression() {
        let baseline = data(Kind::Cpu, &[cpu_row(&[])]).unwrap();
        let current = data(
            Kind::Cpu,
            &[cpu_row(&[
                ("state_hash", "1123456789abcdef"),
                ("contacts", "4"),
            ])],
        )
        .unwrap();
        let mut out = Vec::new();
        assert!(!compare(&baseline, &current, THRESHOLDS, &mut out).unwrap());
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("WORK CHANGED state_hash"));
        assert!(out.contains("WORK CHANGED contacts"));
        assert!(!out.contains("tick_p95_ms"));
    }

    #[test]
    fn rejects_incompatible_or_invalid_csv() {
        let baseline = data(Kind::Cpu, &[cpu_row(&[])]).unwrap();
        let different = data(Kind::Cpu, &[cpu_row(&[("seed", "42")])]).unwrap();
        assert!(compare(&baseline, &different, THRESHOLDS, &mut Vec::new()).is_err());
        assert!(data(Kind::Cpu, &[cpu_row(&[]), cpu_row(&[])]).is_err());
        assert!(data(Kind::Cpu, &[cpu_row(&[("tick_p50_ms", "NaN")])]).is_err());
        assert!(data(Kind::Offscreen, &[gpu_row("test", "5")]).is_err());

        let first = data(Kind::Offscreen, &[gpu_row("\"GPU, \"\"Name\"\"\"", "0")]).unwrap();
        let duplicate = data(Kind::Offscreen, &[gpu_row("\"GPU, \"\"Name\"\"\"", "0")]).unwrap();
        let other = data(Kind::Offscreen, &[gpu_row("Different", "0")]).unwrap();
        let timed = data(Kind::Offscreen, &[gpu_row("\"GPU, \"\"Name\"\"\"", "10")]).unwrap();
        let mut out = Vec::new();
        assert!(compare(&first, &duplicate, THRESHOLDS, &mut out).unwrap());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("comparing CPU only")
        );
        assert!(compare(&first, &timed, THRESHOLDS, &mut Vec::new()).is_err());
        assert!(compare(&first, &other, THRESHOLDS, &mut Vec::new()).is_err());
        assert!(compare(&baseline, &first, THRESHOLDS, &mut Vec::new()).is_err());
    }

    #[test]
    fn csv_parser_handles_quoted_newlines_empty_fields_and_crlf() {
        assert_eq!(
            csv_records("a,b,\r\n\"x\r\ny\",\"\"\"\",").unwrap(),
            [vec!["a", "b", ""], vec!["x\r\ny", "\"", ""],]
        );
        assert!(csv_records("a,\"broken").is_err());
        assert!(csv_records("\"x\"garbage").is_err());
    }
}
