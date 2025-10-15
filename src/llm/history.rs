use crate::llm::LlmPostProcessSettings;
use crate::utils::app_config_dir;
use chrono::Local;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub const HISTORY_FILENAME: &str = "llm_history.yaml";
pub const MAX_HISTORY_ENTRIES: usize = 20;

static HISTORY_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmHistoryEntry {
    pub timestamp: String,
    pub transcript: String,
    pub llm_output: String,
    pub truncated_input: bool,
    pub llm_latency_ms: u64,
    pub settings: LlmHistorySettingsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmHistorySettingsSnapshot {
    pub api_base_url: String,
    pub model: String,
    pub mode_id: String,
    pub mode_label: String,
    pub language_override: Option<String>,
    pub custom_prompt_system: Option<String>,
    pub custom_prompt_user: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct HistorySaveOutcome {
    pub total_entries: usize,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<LlmHistoryEntry>,
}

fn history_path() -> PathBuf {
    app_config_dir().join(HISTORY_FILENAME)
}

fn mode_label(snapshot: &LlmPostProcessSettings) -> String {
    if snapshot.mode_id == crate::llm::PRESET_ID_FORMAT {
        "format".to_string()
    } else if snapshot.mode_id == crate::llm::PRESET_ID_SUMMARY {
        "summary".to_string()
    } else {
        snapshot
            .custom_prompts
            .iter()
            .find(|p| p.id == snapshot.mode_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| snapshot.mode_id.clone())
    }
}

fn build_settings_snapshot(settings: &LlmPostProcessSettings) -> LlmHistorySettingsSnapshot {
    let custom_prompts = &settings.custom_prompts;
    let (custom_system, custom_user) =
        if let Some(custom) = custom_prompts.iter().find(|p| p.id == settings.mode_id) {
            (
                custom.system_prompt.clone(),
                Some(custom.user_prompt.clone()),
            )
        } else if settings.mode_id == crate::llm::MODE_ID_CUSTOM_DRAFT {
            (
                if settings.custom_prompt_system.trim().is_empty() {
                    None
                } else {
                    Some(settings.custom_prompt_system.clone())
                },
                if settings.custom_prompt.trim().is_empty() {
                    None
                } else {
                    Some(settings.custom_prompt.clone())
                },
            )
        } else {
            (None, None)
        };

    LlmHistorySettingsSnapshot {
        api_base_url: settings.effective_base_url(),
        model: settings.model.clone(),
        mode_id: settings.mode_id.clone(),
        mode_label: mode_label(settings),
        language_override: settings.language_override.clone(),
        custom_prompt_system: custom_system,
        custom_prompt_user: custom_user,
    }
}

pub fn record_entry(
    transcript: &str,
    llm_output: &str,
    truncated_input: bool,
    llm_latency_ms: u128,
    settings: &LlmPostProcessSettings,
) -> anyhow::Result<HistorySaveOutcome> {
    let _guard = HISTORY_LOCK.lock().unwrap();

    let path = history_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = if path.exists() {
        let yaml = fs::read_to_string(&path)?;
        serde_yaml::from_str::<HistoryFile>(&yaml).unwrap_or_default()
    } else {
        HistoryFile::default()
    };

    let mut entries = existing.entries;
    let entry = LlmHistoryEntry {
        timestamp: Local::now().to_rfc3339(),
        transcript: transcript.to_string(),
        llm_output: llm_output.to_string(),
        truncated_input,
        llm_latency_ms: llm_latency_ms.min(u64::MAX as u128) as u64,
        settings: build_settings_snapshot(settings),
    };
    entries.push(entry);
    if entries.len() > MAX_HISTORY_ENTRIES {
        let remove_count = entries.len() - MAX_HISTORY_ENTRIES;
        entries.drain(0..remove_count);
    }
    let total_entries = entries.len();

    let file = HistoryFile { entries };
    // Keep newest entries last on disk for chronological order
    let yaml = serde_yaml::to_string(&file)?;
    let tmp_path = path.with_extension("yaml.tmp");
    let mut fh = fs::File::create(&tmp_path)?;
    fh.write_all(yaml.as_bytes())?;
    fh.flush()?;
    fs::rename(tmp_path, path)?;
    Ok(HistorySaveOutcome { total_entries })
}

pub fn load_entries() -> anyhow::Result<Vec<LlmHistoryEntry>> {
    let path = history_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let yaml = fs::read_to_string(&path)?;
    let file = serde_yaml::from_str::<HistoryFile>(&yaml).unwrap_or_default();
    Ok(file.entries)
}

pub fn history_modified_time() -> Option<std::time::SystemTime> {
    let path = history_path();
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn history_file_path() -> PathBuf {
    history_path()
}
