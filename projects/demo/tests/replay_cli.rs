use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct ReplayFile(PathBuf);

impl Drop for ReplayFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir(self.0.parent().unwrap());
    }
}

fn demo(flag: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fate_demo"))
        .arg(flag)
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn records_and_inspects_a_showcase_replay_without_opening_a_window() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("fate-replay-{}-{stamp}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let sample = ReplayFile(dir.join("sample replay.frep"));
    let path = &sample.0;
    let recorded = demo("--record-replay", path);
    assert!(recorded.status.success(), "{recorded:?}");
    let mut bytes = fs::read(path).unwrap();
    assert_eq!(demo("--record-replay", path).status.code(), Some(2));
    assert_eq!(fs::read(path).unwrap(), bytes);

    let inspected = demo("--inspect-replay", path);
    assert!(inspected.status.success(), "{inspected:?}");
    let text = String::from_utf8(inspected.stdout).unwrap();
    assert!(text.contains("ticks 0..=240 (240 inputs)"));
    assert!(text.contains("Verified; final checksum"));

    // Keep both frames structurally valid but change their expected hashes.
    let state_len = u32::from_le_bytes(bytes[6..10].try_into().unwrap()) as usize;
    let frame_start = 14 + state_len;
    for frame in [4, 19] {
        bytes[frame_start + frame * 11 + 3] ^= 1;
    }
    fs::write(path, &bytes).unwrap();
    let mismatch = demo("--inspect-replay", path);
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(
        String::from_utf8(mismatch.stderr)
            .unwrap()
            .contains("replay diverged at tick 5:")
    );

    fs::write(path, &bytes[..bytes.len() - 1]).unwrap();
    let invalid = demo("--inspect-replay", path);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8(invalid.stderr)
            .unwrap()
            .contains("truncated replay data")
    );
}
