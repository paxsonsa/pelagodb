use pelago_storage::failpoints;
use pelago_storage::{FaultMode, FaultProfile, SimulationTrace};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .unwrap_or_else(|_| "/usr/local/etc/foundationdb/fdb.cluster".to_string())
}

pub fn unique_context(prefix: &str, seed: u64) -> (String, String) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic after epoch")
        .as_micros();
    (format!("{}_{}_{}", prefix, seed, ts), "default".to_string())
}

pub fn artifact_dir() -> PathBuf {
    let base = PathBuf::from(".tmp/simulation");
    fs::create_dir_all(&base).expect("artifact dir should be creatable");
    base
}

pub fn trace_path(scenario: &str, seed: u64) -> PathBuf {
    artifact_dir().join(format!("{}_{}.json", scenario, seed))
}

pub fn maybe_save_trace(path: &Path, trace: &SimulationTrace) {
    if std::env::var("PELAGO_SIM_SAVE_TRACE")
        .ok()
        .as_deref()
        .unwrap_or("1")
        == "0"
    {
        return;
    }
    let bytes = serde_json::to_vec_pretty(trace).expect("trace should serialize");
    fs::write(path, bytes).expect("trace file should write");
}

pub fn load_replay_trace(path: &Path) -> SimulationTrace {
    let bytes = fs::read(path).expect("replay trace should read");
    serde_json::from_slice(&bytes).expect("replay trace should parse")
}

pub fn apply_fault_profile(profile: &FaultProfile) {
    failpoints::clear_all();
    for (name, mode) in &profile.failpoints {
        match mode {
            FaultMode::Off => failpoints::clear(name),
            FaultMode::ReturnError { max_triggers } => match max_triggers {
                Some(count) => failpoints::configure_return_error_count(name.clone(), *count),
                None => failpoints::configure_return_error(name.clone()),
            },
        }
    }
}
