use std::{collections::BTreeSet, path::PathBuf};

use crate::settings::{Preset, Settings};

pub enum Launch {
    Help,
    Live {
        settings: Settings,
        timing: bool,
        smoke: bool,
    },
    Headless(Bench),
    Offscreen(GpuBench),
}

pub struct GpuBench {
    pub preset: Preset,
    pub cases: Vec<Case>,
    pub warmup: usize,
    pub frames: usize,
    pub size: (u32, u32),
    pub csv: Option<PathBuf>,
}

pub struct Bench {
    pub preset: Preset,
    pub cases: Vec<Case>,
    pub warmup: usize,
    pub ticks: usize,
    pub csv: Option<PathBuf>,
}

pub struct Case {
    pub settings: Settings,
    pub swept: Option<(&'static str, u32)>,
}

const MAX_SAMPLES: usize = 100_000;
const MAX_CASES: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Live,
    Cpu,
    Offscreen,
}

fn offscreen_number(name: &str) -> bool {
    matches!(name, "candidates" | "detail" | "batch")
}

pub fn help() {
    println!(
        "fate_polygon [--dbg] [--preset NAME] [--set NAME=VALUE]...\n\
         fate_polygon [--dbg] --smoke\n\
         fate_polygon --headless [--preset NAME] [--set NAME=VALUE]...\n\
         \t[--sweep NAME=VALUE,VALUE,...] [--warmup N] [--ticks N] [--csv PATH]\n\
         fate_polygon --offscreen [--preset NAME] [--size WIDTHxHEIGHT] [--frames N] [--warmup N]\n\
         \t[--set NAME=VALUE]... [--sweep NAME=VALUE,VALUE,...] [--csv PATH]\n\n\
         Presets: mixed (default), render, physics, ai. Headless measures CPU lab work only.\n\
         Default headless run: 120 warmup and 600 measured ticks. CSV paths must not exist.\n\
         Offscreen defaults: render preset, 960x540, 30 warmup, 120 measured frames.\n\
         Offscreen accepts candidates, detail, batch and cull. It requires a GPU adapter.\n\
         Sweep one numeric setting, at most {MAX_CASES} distinct values.\n\
         Toggles accept true or false. Only physics and ai apply headlessly:\n\
         \t{}\n\
         Numeric settings:",
        Settings::TOGGLES.join(", ")
    );
    for spec in Settings::NUMBERS {
        println!(
            "\t{:<15} {:>3}..={:<7} {}",
            spec.name,
            spec.min,
            spec.max,
            if spec.cpu {
                "live and CPU lab"
            } else if offscreen_number(spec.name) {
                "live and offscreen"
            } else {
                "live only"
            }
        );
    }
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Launch, String> {
    let mut args = args.into_iter();
    let mut scope = Scope::Live;
    let mut timing = false;
    let mut smoke = false;
    let mut preset = None;
    let mut overrides = Vec::new();
    let mut sweep = None;
    let mut ticks = None;
    let mut frames = None;
    let mut size = None;
    let mut warmup = None;
    let mut csv = None;
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(Launch::Help),
            "--dbg" if !timing => timing = true,
            "--smoke" if !smoke => smoke = true,
            "--headless" if scope == Scope::Live => scope = Scope::Cpu,
            "--offscreen" if scope == Scope::Live => scope = Scope::Offscreen,
            "--preset" if preset.is_none() => {
                let name = value()?;
                preset =
                    Some(Preset::parse(&name).ok_or_else(|| format!("unknown preset '{name}'"))?);
            }
            "--set" => overrides.push(value()?),
            "--sweep" if sweep.is_none() => sweep = Some(value()?),
            "--ticks" if ticks.is_none() => ticks = Some(parse_samples(&value()?, 1, "--ticks")?),
            "--frames" if frames.is_none() => {
                frames = Some(parse_samples(&value()?, 1, "--frames")?)
            }
            "--size" if size.is_none() => size = Some(parse_size(&value()?)?),
            "--warmup" if warmup.is_none() => {
                warmup = Some(parse_samples(&value()?, 0, "--warmup")?)
            }
            "--csv" if csv.is_none() => csv = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown or repeated option '{arg}'")),
        }
    }

    if scope != Scope::Live && (timing || smoke) {
        return Err("--dbg and --smoke are live-only options".to_owned());
    }
    if scope == Scope::Live && (sweep.is_some() || warmup.is_some() || csv.is_some()) {
        return Err("--sweep, --warmup and --csv require --headless or --offscreen".to_owned());
    }
    if ticks.is_some() && scope != Scope::Cpu {
        return Err("--ticks requires --headless".to_owned());
    }
    if (frames.is_some() || size.is_some()) && scope != Scope::Offscreen {
        return Err("--frames and --size require --offscreen".to_owned());
    }
    if smoke && (preset.is_some() || !overrides.is_empty()) {
        return Err("--smoke runs its own presets and does not take overrides".to_owned());
    }

    let preset = preset.unwrap_or(if scope == Scope::Offscreen {
        Preset::Render
    } else {
        Preset::Mixed
    });
    let mut base = Settings::preset(preset);
    for item in overrides {
        let (name, value) = assignment(&item)?;
        if Settings::TOGGLES.contains(&name) {
            if (scope == Scope::Cpu && !matches!(name, "physics" | "ai"))
                || (scope == Scope::Offscreen && name != "cull")
            {
                return Err(format!("'{name}' does not apply to this benchmark"));
            }
            let value = value
                .parse::<bool>()
                .map_err(|_| format!("'{name}' expects true or false"))?;
            base.apply_toggle(name, value);
        } else {
            let (name, value) = setting_value(name, value, scope)?;
            base.apply_number(name, f64::from(value));
        }
    }
    if scope == Scope::Live {
        return Ok(Launch::Live {
            settings: base,
            timing,
            smoke,
        });
    }
    if scope == Scope::Cpu && !base.physics && !base.ai {
        return Err("headless mode requires physics or AI; rendering is not measured".to_owned());
    }
    if scope == Scope::Offscreen && !base.render {
        return Err("offscreen mode requires a rendering preset".to_owned());
    }

    let mut cases = Vec::new();
    if let Some(sweep) = sweep {
        let (name, values) = assignment(&sweep)?;
        let mut seen = BTreeSet::new();
        for value in values.split(',') {
            let (name, value) = setting_value(name, value, scope)?;
            if cases.len() == MAX_CASES || !seen.insert(value) {
                return Err(format!("sweep allows at most {MAX_CASES} distinct values"));
            }
            let mut settings = base;
            settings.apply_number(name, f64::from(value));
            cases.push(Case {
                settings,
                swept: Some((name, value)),
            });
        }
    } else {
        cases.push(Case {
            settings: base,
            swept: None,
        });
    }
    if scope == Scope::Offscreen {
        return Ok(Launch::Offscreen(GpuBench {
            preset,
            cases,
            warmup: warmup.unwrap_or(30),
            frames: frames.unwrap_or(120),
            size: size.unwrap_or((960, 540)),
            csv,
        }));
    }
    Ok(Launch::Headless(Bench {
        preset,
        cases,
        ticks: ticks.unwrap_or(600),
        warmup: warmup.unwrap_or(120),
        csv,
    }))
}

fn parse_samples(text: &str, min: usize, option: &str) -> Result<usize, String> {
    text.parse()
        .ok()
        .filter(|samples| (min..=MAX_SAMPLES).contains(samples))
        .ok_or_else(|| format!("{option} expects an integer in {min}..={MAX_SAMPLES}"))
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let side = |text: &str| text.parse::<u32>().ok().filter(|n| (1..=4096).contains(n));
    text.split_once('x')
        .and_then(|(width, height)| Some((side(width)?, side(height)?)))
        .ok_or_else(|| "--size expects WIDTHxHEIGHT, each in 1..=4096".to_owned())
}

fn assignment(text: &str) -> Result<(&str, &str), String> {
    text.split_once('=')
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .ok_or_else(|| format!("expected NAME=VALUE, got '{text}'"))
}

fn setting_value(name: &str, text: &str, scope: Scope) -> Result<(&'static str, u32), String> {
    let spec = Settings::NUMBERS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| format!("unknown numeric setting '{name}'"))?;
    if (scope == Scope::Cpu && !spec.cpu) || (scope == Scope::Offscreen && !offscreen_number(name))
    {
        return Err(format!("'{name}' does not apply to this benchmark"));
    }
    let value = text
        .parse()
        .ok()
        .filter(|v| (spec.min..=spec.max).contains(v))
        .ok_or_else(|| format!("'{name}' expects an integer in {}..={}", spec.min, spec.max))?;
    Ok((spec.name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(args: &[&str]) -> Result<Launch, String> {
        parse(args.iter().map(ToString::to_string))
    }

    #[test]
    fn applies_presets_overrides_and_sweep_in_that_order() {
        let Launch::Headless(bench) = launch(&[
            "--headless",
            "--set",
            "passes=3",
            "--preset",
            "physics",
            "--sweep",
            "bodies=384,64,128",
            "--ticks",
            "12",
            "--warmup",
            "0",
        ])
        .unwrap() else {
            panic!("headless expected")
        };
        assert_eq!((bench.warmup, bench.ticks), (0, 12));
        assert_eq!(
            bench
                .cases
                .iter()
                .map(|c| c.settings.bodies)
                .collect::<Vec<_>>(),
            [384, 64, 128]
        );
        assert!(
            bench
                .cases
                .iter()
                .all(|c| c.settings.passes == 3 && !c.settings.ai)
        );
        assert_eq!(bench.cases[1].swept, Some(("bodies", 64)));
    }

    #[test]
    fn rejects_invalid_or_misleading_runs() {
        for args in [
            &["--headless", "--dbg"][..],
            &["--sweep", "bodies=1,2"],
            &["--headless", "--sweep", "bodies=1,1"],
            &["--headless", "--set", "bodies=385"],
            &["--headless", "--set", "detail=15"],
            &["--headless", "--preset", "render"],
            &["--headless", "--ticks", "0"],
            &["--headless", "--sweep", "agents=1,"],
        ] {
            assert!(launch(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn live_accepts_the_same_setting_names() {
        let Launch::Live { settings, .. } = launch(&[
            "--preset",
            "ai",
            "--set",
            "agents=400",
            "--set",
            "detail=30",
            "--set",
            "render=true",
        ])
        .unwrap() else {
            panic!("live expected")
        };
        assert_eq!((settings.agents, settings.detail), (400, 30));
        assert!(settings.render && settings.ai && !settings.physics);
    }

    #[test]
    fn offscreen_has_its_own_frame_budget_and_scope() {
        let Launch::Offscreen(bench) = launch(&[
            "--offscreen",
            "--size",
            "320x240",
            "--frames",
            "8",
            "--warmup",
            "2",
            "--set",
            "cull=true",
            "--sweep",
            "detail=3,48",
        ])
        .unwrap() else {
            panic!("offscreen expected")
        };
        assert_eq!(bench.preset.name(), "render");
        assert_eq!((bench.size, bench.frames, bench.warmup), ((320, 240), 8, 2));
        assert_eq!(bench.cases[1].settings.detail, 48);
        assert!(bench.cases[1].settings.cull);
        for args in [
            &["--offscreen", "--headless"][..],
            &["--offscreen", "--ticks", "8"],
            &["--offscreen", "--size", "0x256"],
            &["--offscreen", "--set", "frame_hz=30"],
            &["--offscreen", "--sweep", "agents=10,20"],
            &["--headless", "--frames", "60"],
        ] {
            assert!(launch(args).is_err(), "{args:?}");
        }
    }
}
