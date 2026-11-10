use std::process::ExitCode;

use fate_sim_bench::benchmark_scenarios;

fn main() -> ExitCode {
    match parse_tick_count(std::env::args().skip(1)) {
        Ok(ticks) => match benchmark_scenarios(ticks) {
            Ok(reports) => {
                for report in reports {
                    println!("{}", report.summary());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: cargo run -p fate_sim_bench -- [--ticks <positive integer>]");
            ExitCode::FAILURE
        }
    }
}

fn parse_tick_count(mut arguments: impl Iterator<Item = String>) -> Result<usize, String> {
    let Some(flag) = arguments.next() else {
        return Ok(10_000);
    };
    if flag != "--ticks" {
        return Err(format!("unknown argument: {flag}"));
    }
    let value = arguments
        .next()
        .ok_or_else(|| "--ticks requires a positive integer".to_owned())?;
    if arguments.next().is_some() {
        return Err("unexpected extra argument".to_owned());
    }
    let ticks = value
        .parse::<usize>()
        .map_err(|_| format!("invalid tick count: {value}"))?;
    if ticks == 0 {
        return Err("tick count must be greater than zero".to_owned());
    }
    Ok(ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<usize, String> {
        parse_tick_count(values.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn defaults_and_accepts_explicit_tick_counts() {
        assert_eq!(args(&[]).unwrap(), 10_000);
        assert_eq!(args(&["--ticks", "120"]).unwrap(), 120);
    }

    #[test]
    fn rejects_invalid_or_ambiguous_arguments() {
        assert!(args(&["--ticks", "0"]).is_err());
        assert!(args(&["--ticks", "abc"]).is_err());
        assert!(args(&["--other"]).is_err());
        assert!(args(&["--ticks", "10", "extra"]).is_err());
    }
}
