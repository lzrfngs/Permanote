use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub vault_root: Option<String>,
    pub permanote_mode: String, // "color" | "label"
    pub theme: String,          // "light" | "dark" | "system"
    pub permanote_order: Vec<String>, // manual sort order, by permanote id
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vault_root: None,
            permanote_mode: "color".into(),
            theme: "system".into(),
            permanote_order: Vec::new(),
        }
    }
}

static CACHE: OnceLock<Mutex<Settings>> = OnceLock::new();

fn cache() -> &'static Mutex<Settings> {
    CACHE.get_or_init(|| Mutex::new(load_from_disk()))
}

pub fn settings_path() -> Result<PathBuf, String> {
    let dir = dirs::config_dir()
        .ok_or_else(|| "Could not locate config dir".to_string())?
        .join("permanote");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

fn load_from_disk() -> Settings {
    let Ok(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn get() -> Settings {
    cache().lock().unwrap().clone()
}

pub fn save(new: Settings) -> Result<(), String> {
    let path = settings_path()?;
    let json = serde_json::to_string_pretty(&new).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    *cache().lock().unwrap() = new;
    Ok(())
}

/// True when no settings file exists yet, or when vault_root is unset.
pub fn is_first_run() -> bool {
    let Ok(path) = settings_path() else {
        return true;
    };
    if !path.exists() {
        return true;
    }
    get()
        .vault_root
        .as_deref()
        .map(|s| s.is_empty())
        .unwrap_or(true)
}
