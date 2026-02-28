use pelago_core::PelagoError;

#[cfg(feature = "failpoints")]
use std::collections::HashMap;
#[cfg(feature = "failpoints")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "failpoints")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailAction {
    Off,
    ReturnError { remaining: Option<u32> },
}

#[cfg(feature = "failpoints")]
impl FailAction {
    fn should_fire(&mut self) -> bool {
        match self {
            FailAction::Off => false,
            FailAction::ReturnError {
                remaining: Some(left),
            } => {
                if *left == 0 {
                    false
                } else {
                    *left -= 1;
                    true
                }
            }
            FailAction::ReturnError { remaining: None } => true,
        }
    }
}

#[cfg(feature = "failpoints")]
fn registry() -> &'static Mutex<HashMap<String, FailAction>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, FailAction>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "failpoints")]
pub fn configure(name: impl Into<String>, action: FailAction) {
    let mut map = registry().lock().expect("failpoint registry poisoned");
    map.insert(name.into(), action);
}

#[cfg(feature = "failpoints")]
pub fn configure_return_error(name: impl Into<String>) {
    configure(name, FailAction::ReturnError { remaining: None });
}

#[cfg(feature = "failpoints")]
pub fn configure_return_error_count(name: impl Into<String>, count: u32) {
    configure(
        name,
        FailAction::ReturnError {
            remaining: Some(count),
        },
    );
}

#[cfg(feature = "failpoints")]
pub fn clear(name: &str) {
    let mut map = registry().lock().expect("failpoint registry poisoned");
    map.remove(name);
}

#[cfg(feature = "failpoints")]
pub fn clear_all() {
    let mut map = registry().lock().expect("failpoint registry poisoned");
    map.clear();
}

pub fn action_label(action: &str) -> String {
    action
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(feature = "failpoints")]
pub fn inject(name: &str) -> Result<(), PelagoError> {
    let mut map = registry().lock().expect("failpoint registry poisoned");
    let should_fail = map.get_mut(name).map(|v| v.should_fire()).unwrap_or(false);
    drop(map);

    if should_fail {
        return Err(PelagoError::Internal(format!(
            "Injected failpoint triggered: {}",
            name
        )));
    }

    Ok(())
}

#[cfg(not(feature = "failpoints"))]
pub fn inject(_name: &str) -> Result<(), PelagoError> {
    Ok(())
}

pub fn inject_action(prefix: &str, action: &str) -> Result<(), PelagoError> {
    let point = format!("{}.{}", prefix, action_label(action));
    inject(&point)
}

#[cfg(not(feature = "failpoints"))]
pub fn configure_return_error(_name: impl Into<String>) {}

#[cfg(not(feature = "failpoints"))]
pub fn configure_return_error_count(_name: impl Into<String>, _count: u32) {}

#[cfg(not(feature = "failpoints"))]
pub fn clear(_name: &str) {}

#[cfg(not(feature = "failpoints"))]
pub fn clear_all() {}
