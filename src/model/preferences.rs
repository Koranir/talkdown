//! Persisted application preferences and launch-time model selection.

use crate::checker::CheckingProvider;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{
    DEFAULT_MODEL_NAME, DEFAULT_TEXT_SCALE_PERCENT, DEFAULT_UI_SCALE_PERCENT,
    MAX_TEXT_SCALE_PERCENT, MAX_UI_SCALE_PERCENT, MIN_TEXT_SCALE_PERCENT, MIN_UI_SCALE_PERCENT,
};

const SETTINGS_FILE: &str = "settings.json";
const MODEL_OVERRIDE_ENVIRONMENT: &str = "TALKDOWN_WHISPER_MODEL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    Environment,
    Saved,
    Default,
    Unset,
}

#[derive(Debug, Clone)]
pub struct InitialModel {
    pub path: Option<PathBuf>,
    pub source: ModelSource,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    pub speech_model_path: Option<PathBuf>,
    pub checking_provider: CheckingProvider,
    pub codex_model: Option<String>,
    pub text_scale_percent: u16,
    pub ui_scale_percent: u16,
    pub word_wrap: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            speech_model_path: None,
            checking_provider: CheckingProvider::default(),
            codex_model: None,
            text_scale_percent: DEFAULT_TEXT_SCALE_PERCENT,
            ui_scale_percent: DEFAULT_UI_SCALE_PERCENT,
            word_wrap: true,
        }
    }
}

/// Resolves the launch-time speech model in precedence order.
///
/// An environment override wins over persisted preferences. If neither is
/// configured, the installed-default location is selected when its file exists.
/// A settings read error is retained as a warning while that fallback still
/// proceeds.
pub fn initial_model() -> InitialModel {
    if let Some(path) = environment_model_path() {
        return selected_model(path, ModelSource::Environment);
    }

    match load_preferences().map(|preferences| preferences.speech_model_path) {
        Ok(Some(path)) => selected_model(path, ModelSource::Saved),
        Ok(None) => installed_default_or_unset(None),
        Err(error) => installed_default_or_unset(Some(error)),
    }
}

pub fn default_model_path() -> Result<PathBuf, String> {
    Ok(application_paths()?.default_model)
}

pub fn load_preferences() -> Result<AppPreferences, String> {
    let path = application_paths()?.settings;
    load_preferences_at(&path)
}

#[cfg(not(test))]
pub fn save_preferences(preferences: &AppPreferences) -> Result<(), String> {
    let path = application_paths()?.settings;
    save_preferences_at(&path, preferences)
}

fn environment_model_path() -> Option<PathBuf> {
    std::env::var_os(MODEL_OVERRIDE_ENVIRONMENT).map(PathBuf::from)
}

fn selected_model(path: PathBuf, source: ModelSource) -> InitialModel {
    InitialModel {
        path: Some(path),
        source,
        warning: None,
    }
}

fn installed_default_or_unset(warning: Option<String>) -> InitialModel {
    let path = default_model_path().ok().filter(|path| path.is_file());
    let source = if path.is_some() {
        ModelSource::Default
    } else {
        ModelSource::Unset
    };

    InitialModel {
        path,
        source,
        warning,
    }
}

struct ApplicationPaths {
    settings: PathBuf,
    default_model: PathBuf,
}

fn application_paths() -> Result<ApplicationPaths, String> {
    let directories = project_directories()?;
    Ok(ApplicationPaths {
        settings: directories.config_dir().join(SETTINGS_FILE),
        default_model: directories
            .data_dir()
            .join("models")
            .join(DEFAULT_MODEL_NAME),
    })
}

fn project_directories() -> Result<ProjectDirs, String> {
    ProjectDirs::from("dev", "Talkdown", "Talkdown")
        .ok_or_else(|| "the operating system did not provide an application-data directory".into())
}

pub(super) fn load_preferences_at(path: &Path) -> Result<AppPreferences, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(AppPreferences::default());
        }
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };

    serde_json::from_slice::<AppPreferences>(&bytes)
        .map(normalize_loaded_preferences)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn normalize_loaded_preferences(mut preferences: AppPreferences) -> AppPreferences {
    if preferences
        .codex_model
        .as_ref()
        .is_some_and(|model| model.trim().is_empty())
    {
        preferences.codex_model = None;
    }
    preferences.text_scale_percent = preferences
        .text_scale_percent
        .clamp(MIN_TEXT_SCALE_PERCENT, MAX_TEXT_SCALE_PERCENT);
    preferences.ui_scale_percent = preferences
        .ui_scale_percent
        .clamp(MIN_UI_SCALE_PERCENT, MAX_UI_SCALE_PERCENT);
    preferences
}

pub(super) fn save_preferences_at(
    settings_path: &Path,
    preferences: &AppPreferences,
) -> Result<(), String> {
    let parent = settings_path
        .parent()
        .ok_or_else(|| "the Talkdown settings path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create the settings directory: {error}"))?;

    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|error| format!("could not encode settings: {error}"))?;
    let temporary = temporary_settings_path(settings_path);
    write_atomic(&temporary, settings_path, &bytes)
        .map_err(|error| format!("could not save model settings: {error}"))
}

pub(super) fn temporary_settings_path(settings_path: &Path) -> PathBuf {
    append_suffix(settings_path, ".part")
}

fn write_atomic(temporary: &Path, destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
