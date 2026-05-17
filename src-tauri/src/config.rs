use serde::{Deserialize, Serialize};
use tauri::Wry;
use tauri_plugin_store::Store;

const CONFIG_KEY: &str = "app_config";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub poll_interval_ms: u64,
    pub debug_mode: bool,
    pub auto_start: bool,
    pub blacklist: Vec<String>,
    pub whitelist_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 250,
            debug_mode: false,
            auto_start: true,
            blacklist: Vec::new(),
            whitelist_mode: false,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_interval_ms < 50 || self.poll_interval_ms > 1000 {
            return Err("poll_interval_ms must be between 50 and 1000".to_string());
        }
        Ok(())
    }

    pub fn is_app_allowed(&self, process_name: &str) -> bool {
        let name = process_name.to_lowercase();
        let list: Vec<String> = self.blacklist.iter().map(|s| s.to_lowercase()).collect();
        if self.whitelist_mode {
            list.contains(&name)
        } else {
            !list.contains(&name)
        }
    }
}

pub fn init_default_config(store: &Store<Wry>) {
    if store.get(CONFIG_KEY).is_none() {
        let default = serde_json::to_value(AppConfig::default()).unwrap();
        store.set(CONFIG_KEY, default);
        let _ = store.save();
    }
}

pub fn load_config(store: &Store<Wry>) -> AppConfig {
    store
        .get(CONFIG_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub fn save_config(store: &Store<Wry>, config: &AppConfig) -> Result<(), String> {
    config.validate()?;
    let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    store.set(CONFIG_KEY, value);
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}
