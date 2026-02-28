use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulationConfig {
    pub seed: u64,
    pub steps: u32,
    pub workers: u16,
    pub scenario: String,
    pub fail_profile: FaultProfile,
}

impl SimulationConfig {
    pub fn new(seed: u64, steps: u32, workers: u16, scenario: impl Into<String>) -> Self {
        Self {
            seed,
            steps,
            workers,
            scenario: scenario.into(),
            fail_profile: FaultProfile::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FaultProfile {
    pub failpoints: HashMap<String, FaultMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FaultMode {
    Off,
    ReturnError {
        #[serde(default)]
        max_triggers: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScenarioOperation {
    CreateNode {
        worker: u16,
        label: String,
        age: i64,
    },
    UpdateNode {
        worker: u16,
        slot: usize,
        age: i64,
    },
    DeleteNode {
        worker: u16,
        slot: usize,
    },
    CreateEdge {
        worker: u16,
        source_slot: usize,
        target_slot: usize,
    },
    DeleteEdge {
        worker: u16,
        source_slot: usize,
        target_slot: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioEvent {
    pub step: u32,
    pub worker: u16,
    pub operation: String,
    pub status: ScenarioEventStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScenarioEventStatus {
    Applied,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulationTrace {
    pub config: SimulationConfig,
    pub operations: Vec<ScenarioOperation>,
    pub events: Vec<ScenarioEvent>,
}
