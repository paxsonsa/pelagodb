use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct CliConfig {
    pub server: Option<String>,
    pub database: Option<String>,
    pub namespace: Option<String>,
    pub format: Option<String>,
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pelago")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> CliConfig {
    let path = config_path();
    if !path.exists() {
        return CliConfig::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            // Parse TOML config
            if let Ok(doc) = contents.parse::<toml_edit::DocumentMut>() {
                CliConfig {
                    server: doc.get("server").and_then(|v| v.as_str()).map(String::from),
                    database: doc
                        .get("database")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    namespace: doc
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    format: doc
                        .get("format")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                }
            } else {
                CliConfig::default()
            }
        }
        Err(_) => CliConfig::default(),
    }
}
