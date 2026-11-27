//! The *Connect to Claude* screen (docs/ux.md, "Connect to Claude"; decision
//! record 0031): where `kadr-mcp` is, Claude Desktop's configuration, and the
//! folders Claude may use.
//!
//! - The folders live in `claude-access.json` in the app's configuration folder;
//!   `kadr-mcp` reads it before every call (`--kadr-settings`), so a change
//!   applies at once.
//! - **Connect** adds or replaces only the `kadr` entry of `mcpServers` in Claude
//!   Desktop's configuration, keeps everything else and a backup of the previous
//!   file, and never touches a file that is not valid JSON.

use std::fs;
use std::path::{Path, PathBuf};

use kadr_tools::{CLAUDE_ACCESS_FILE_NAME, ClaudeAccessSettings};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::command_error::{CommandError, CommandResult, ErrorCode};

/// The name of Claude Desktop's configuration file.
const CLAUDE_DESKTOP_CONFIG_FILE_NAME: &str = "claude_desktop_config.json";
/// The name of Kadr's entry in that file.
const MCP_SERVER_NAME: &str = "kadr";
/// The backup of the configuration made before Kadr changes it.
const BACKUP_EXTENSION: &str = "json.kadr-backup";

/// Whether Claude Desktop knows Kadr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeDesktopConnection {
    /// The configuration starts this app's `kadr-mcp` with this app's settings.
    Connected,
    /// No `kadr` entry yet (or no configuration file).
    NotConnected,
    /// A `kadr` entry starts another program, or without this app's settings
    /// (set up by hand, or by another copy of Kadr).
    OtherKadr,
    /// The file is not valid JSON: Kadr will not change it.
    Unreadable,
}

/// One allowed folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedFolderView {
    /// The folder, as chosen.
    pub path: String,
    /// Whether it exists (a missing one is ignored until it does).
    pub exists: bool,
}

/// What the screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSetupView {
    /// `kadr-mcp`, next to the app's program; `None` when it is missing.
    pub kadr_mcp_path: Option<String>,
    /// Kadr's settings file holding the folders.
    pub settings_file: String,
    /// The folders Claude may use.
    pub allowed_folders: Vec<AllowedFolderView>,
    /// Claude Desktop's configuration file, found or chosen.
    pub claude_desktop_config: String,
    /// Whether that file exists yet.
    pub claude_desktop_config_exists: bool,
    /// Whether Claude Desktop knows Kadr.
    pub connection: ClaudeDesktopConnection,
    /// The command adding Kadr to Claude Code, when `kadr-mcp` is there.
    pub claude_code_command: Option<String>,
}

/// Where things are, for one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeSetupPaths {
    /// The app's configuration folder.
    pub configuration_folder: PathBuf,
    /// `kadr-mcp`, if found.
    pub kadr_mcp: Option<PathBuf>,
    /// Claude Desktop's configuration file.
    pub claude_desktop_config: PathBuf,
}

impl ClaudeSetupPaths {
    /// The paths of this app: `kadr-mcp` next to its own program, Claude
    /// Desktop's configuration where it was found (or `chosen_config`).
    pub fn of_this_app(configuration_folder: PathBuf, chosen_config: Option<PathBuf>) -> Self {
        Self {
            configuration_folder,
            kadr_mcp: kadr_mcp_next_to_this_program(),
            claude_desktop_config: chosen_config.unwrap_or_else(find_claude_desktop_config),
        }
    }

    /// Kadr's settings file.
    pub fn settings_file(&self) -> PathBuf {
        self.configuration_folder.join(CLAUDE_ACCESS_FILE_NAME)
    }
}

/// What the screen shows for `paths`.
pub fn claude_setup_view(paths: &ClaudeSetupPaths) -> ClaudeSetupView {
    let settings_file = paths.settings_file();
    let settings = ClaudeAccessSettings::load(&settings_file).unwrap_or_default();
    let config = &paths.claude_desktop_config;
    ClaudeSetupView {
        kadr_mcp_path: paths
            .kadr_mcp
            .as_ref()
            .map(|path| path.display().to_string()),
        settings_file: settings_file.display().to_string(),
        allowed_folders: settings
            .allowed_folders
            .iter()
            .map(|folder| AllowedFolderView {
                path: folder.display().to_string(),
                exists: folder.is_dir(),
            })
            .collect(),
        claude_desktop_config: config.display().to_string(),
        claude_desktop_config_exists: config.exists(),
        connection: connection_of(config, paths.kadr_mcp.as_deref(), &settings_file),
        claude_code_command: paths.kadr_mcp.as_ref().map(|kadr_mcp| {
            format!(
                "claude mcp add {MCP_SERVER_NAME} -- \"{}\" --kadr-settings \"{}\"",
                kadr_mcp.display(),
                settings_file.display()
            )
        }),
    }
}

/// Adds `folder` to the folders Claude may use.
///
/// # Errors
///
/// When it is not a folder, or the settings cannot be saved.
pub fn add_allowed_folder(paths: &ClaudeSetupPaths, folder: PathBuf) -> CommandResult<()> {
    if !folder.is_dir() {
        return Err(CommandError::new(
            ErrorCode::ClaudeFolderUnusable,
            format!("\"{}\" is not a folder", folder.display()),
        ));
    }
    update_settings(paths, |settings| settings.add_folder(folder))
}

/// Removes `folder` from the folders Claude may use.
///
/// # Errors
///
/// When the settings cannot be saved.
pub fn remove_allowed_folder(paths: &ClaudeSetupPaths, folder: &Path) -> CommandResult<()> {
    update_settings(paths, |settings| settings.remove_folder(folder))
}

fn update_settings(
    paths: &ClaudeSetupPaths,
    change: impl FnOnce(&mut ClaudeAccessSettings),
) -> CommandResult<()> {
    let settings_file = paths.settings_file();
    let mut settings = ClaudeAccessSettings::load(&settings_file).map_err(|error| {
        CommandError::new(ErrorCode::ClaudeSettingsNotSaved, error).about_file(&settings_file)
    })?;
    change(&mut settings);
    settings.save(&settings_file).map_err(|error| {
        CommandError::new(ErrorCode::ClaudeSettingsNotSaved, error).about_file(&settings_file)
    })
}

/// Writes Kadr's entry into Claude Desktop's configuration.
///
/// # Errors
///
/// When `kadr-mcp` is missing, the file is not valid JSON (it is then left as it
/// is), or it cannot be written.
pub fn connect_claude_desktop(paths: &ClaudeSetupPaths) -> CommandResult<()> {
    let Some(kadr_mcp) = &paths.kadr_mcp else {
        return Err(CommandError::new(
            ErrorCode::KadrMcpMissing,
            "kadr-mcp was not found next to the Kadr app",
        ));
    };
    let config_path = &paths.claude_desktop_config;
    let unreadable = |details: String| {
        CommandError::new(ErrorCode::ClaudeConfigUnreadable, details).about_file(config_path)
    };
    let original_text = match fs::read_to_string(config_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(unreadable(error.to_string())),
    };
    let mut config = match original_text.as_deref() {
        Some(text) if !text.trim().is_empty() => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => object,
            Ok(_) => return Err(unreadable("it is not a JSON object".to_string())),
            Err(error) => return Err(unreadable(error.to_string())),
        },
        _ => Map::new(),
    };
    let servers = config
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(unreadable(
            "its \"mcpServers\" is not a JSON object".to_string(),
        ));
    };
    servers.insert(
        MCP_SERVER_NAME.to_string(),
        json!({
            "command": kadr_mcp.display().to_string(),
            "args": ["--kadr-settings", paths.settings_file().display().to_string()],
        }),
    );
    let not_saved = |error: std::io::Error| {
        CommandError::new(ErrorCode::ClaudeConfigNotSaved, error).about_file(config_path)
    };
    if let Some(folder) = config_path.parent() {
        fs::create_dir_all(folder).map_err(not_saved)?;
    }
    if let Some(original_text) = &original_text {
        fs::write(config_path.with_extension(BACKUP_EXTENSION), original_text)
            .map_err(not_saved)?;
    }
    let text =
        serde_json::to_string_pretty(&Value::Object(config)).map_err(CommandError::internal)?;
    let temporary_path = config_path.with_extension("json.kadr-saving");
    fs::write(&temporary_path, text).map_err(not_saved)?;
    fs::rename(&temporary_path, config_path).map_err(not_saved)
}

/// Whether the configuration at `config_path` starts `kadr_mcp` with `settings_file`.
fn connection_of(
    config_path: &Path,
    kadr_mcp: Option<&Path>,
    settings_file: &Path,
) -> ClaudeDesktopConnection {
    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(_) => return ClaudeDesktopConnection::NotConnected,
    };
    if text.trim().is_empty() {
        return ClaudeDesktopConnection::NotConnected;
    }
    let Ok(config) = serde_json::from_str::<Value>(&text) else {
        return ClaudeDesktopConnection::Unreadable;
    };
    let Some(entry) = config
        .get("mcpServers")
        .and_then(|servers| servers.get(MCP_SERVER_NAME))
    else {
        return ClaudeDesktopConnection::NotConnected;
    };
    let starts_this_kadr_mcp = kadr_mcp.is_some_and(|kadr_mcp| {
        entry.get("command").and_then(Value::as_str) == Some(&kadr_mcp.display().to_string())
    });
    let settings_file_text = settings_file.display().to_string();
    let uses_these_settings = entry
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| {
            args.windows(2).any(|pair| {
                pair[0].as_str() == Some("--kadr-settings")
                    && pair[1].as_str() == Some(settings_file_text.as_str())
            })
        });
    if starts_this_kadr_mcp && uses_these_settings {
        ClaudeDesktopConnection::Connected
    } else {
        ClaudeDesktopConnection::OtherKadr
    }
}

/// `kadr-mcp` next to the running program, if it is there.
fn kadr_mcp_next_to_this_program() -> Option<PathBuf> {
    let program = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "kadr-mcp.exe"
    } else {
        "kadr-mcp"
    };
    let candidate = program.with_file_name(name);
    candidate.is_file().then_some(candidate)
}

/// Claude Desktop's configuration file: the most recently changed of the usual
/// places that has one, or the usual place of this system.
fn find_claude_desktop_config() -> PathBuf {
    let candidates = claude_desktop_config_candidates();
    let newest_existing = candidates
        .iter()
        .filter_map(|path| {
            let modified = fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path.clone());
    newest_existing
        .or_else(|| candidates.first().cloned())
        .unwrap_or_else(|| PathBuf::from(CLAUDE_DESKTOP_CONFIG_FILE_NAME))
}

/// Where Claude Desktop keeps its configuration on this system, the usual place
/// first. On Windows, Store installations keep it in their package folder, and
/// some installations in a per-installation folder under `Claude-Data`.
fn claude_desktop_config_candidates() -> Vec<PathBuf> {
    let environment = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut candidates = Vec::new();
    if cfg!(windows) {
        if let Some(roaming) = environment("APPDATA") {
            candidates.push(roaming.join("Claude").join(CLAUDE_DESKTOP_CONFIG_FILE_NAME));
        }
        if let Some(local) = environment("LOCALAPPDATA") {
            candidates.extend(
                subfolders(&local.join("Packages"))
                    .into_iter()
                    .filter(|package| {
                        package
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with("Claude"))
                    })
                    .map(|package| {
                        package
                            .join("LocalCache")
                            .join("Roaming")
                            .join("Claude")
                            .join(CLAUDE_DESKTOP_CONFIG_FILE_NAME)
                    }),
            );
            candidates.extend(
                subfolders(&local.join("Claude-Data"))
                    .into_iter()
                    .map(|folder| folder.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME)),
            );
        }
    } else if let Some(home) = environment("HOME") {
        if cfg!(target_os = "macos") {
            candidates.push(
                home.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join(CLAUDE_DESKTOP_CONFIG_FILE_NAME),
            );
        } else {
            candidates.push(
                home.join(".config")
                    .join("Claude")
                    .join(CLAUDE_DESKTOP_CONFIG_FILE_NAME),
            );
        }
    }
    candidates
}

fn subfolders(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(folder: &Path) -> ClaudeSetupPaths {
        let kadr_mcp = folder.join("app").join("kadr-mcp.exe");
        fs::create_dir_all(kadr_mcp.parent().unwrap()).unwrap();
        fs::write(&kadr_mcp, "").unwrap();
        ClaudeSetupPaths {
            configuration_folder: folder.join("config"),
            kadr_mcp: Some(kadr_mcp),
            claude_desktop_config: folder.join("Claude").join(CLAUDE_DESKTOP_CONFIG_FILE_NAME),
        }
    }

    #[test]
    fn connecting_creates_the_entry_and_keeps_the_rest() {
        let folder = tempfile::tempdir().unwrap();
        let paths = paths_in(folder.path());
        fs::create_dir_all(paths.claude_desktop_config.parent().unwrap()).unwrap();
        let original = r#"{ "preferences": { "theme": "dark" },
            "mcpServers": { "other": { "command": "other.exe" } } }"#;
        fs::write(&paths.claude_desktop_config, original).unwrap();
        assert_eq!(
            claude_setup_view(&paths).connection,
            ClaudeDesktopConnection::NotConnected
        );

        connect_claude_desktop(&paths).unwrap();

        let written: Value =
            serde_json::from_str(&fs::read_to_string(&paths.claude_desktop_config).unwrap())
                .unwrap();
        assert_eq!(written["preferences"]["theme"], "dark");
        assert_eq!(written["mcpServers"]["other"]["command"], "other.exe");
        assert_eq!(written["mcpServers"]["kadr"]["args"][0], "--kadr-settings");
        let backup = paths.claude_desktop_config.with_extension(BACKUP_EXTENSION);
        assert_eq!(fs::read_to_string(backup).unwrap(), original);
        assert_eq!(
            claude_setup_view(&paths).connection,
            ClaudeDesktopConnection::Connected
        );
    }

    #[test]
    fn a_missing_file_is_created_and_an_invalid_one_is_left_alone() {
        let folder = tempfile::tempdir().unwrap();
        let paths = paths_in(folder.path());
        connect_claude_desktop(&paths).unwrap();
        assert!(paths.claude_desktop_config.exists());

        fs::write(&paths.claude_desktop_config, "{ not json").unwrap();
        let error = connect_claude_desktop(&paths).unwrap_err();
        assert_eq!(error.code, ErrorCode::ClaudeConfigUnreadable);
        assert_eq!(
            fs::read_to_string(&paths.claude_desktop_config).unwrap(),
            "{ not json"
        );
        assert_eq!(
            claude_setup_view(&paths).connection,
            ClaudeDesktopConnection::Unreadable
        );
    }

    #[test]
    fn another_kadr_entry_is_told_apart() {
        let folder = tempfile::tempdir().unwrap();
        let paths = paths_in(folder.path());
        fs::create_dir_all(paths.claude_desktop_config.parent().unwrap()).unwrap();
        fs::write(
            &paths.claude_desktop_config,
            r#"{ "mcpServers": { "kadr": { "command": "C:/old/kadr-mcp.exe",
                 "args": ["--allowed-folder", "C:/Videos"] } } }"#,
        )
        .unwrap();
        assert_eq!(
            claude_setup_view(&paths).connection,
            ClaudeDesktopConnection::OtherKadr
        );
    }

    #[test]
    fn folders_are_added_and_removed_in_the_settings() {
        let folder = tempfile::tempdir().unwrap();
        let paths = paths_in(folder.path());
        let videos = folder.path().join("Videos");
        fs::create_dir_all(&videos).unwrap();
        add_allowed_folder(&paths, videos.clone()).unwrap();
        let error = add_allowed_folder(&paths, folder.path().join("missing")).unwrap_err();
        assert_eq!(error.code, ErrorCode::ClaudeFolderUnusable);
        let view = claude_setup_view(&paths);
        assert_eq!(view.allowed_folders.len(), 1);
        assert!(view.allowed_folders[0].exists);
        remove_allowed_folder(&paths, &videos).unwrap();
        assert!(claude_setup_view(&paths).allowed_folders.is_empty());
    }
}
