use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReaderConfig {
    pub show_progress: bool,
    pub discreet_mode: bool,
    pub mouse_support: bool,
    pub text_width: Option<u16>,
    /// Number of simulated agent session rows to render in Agent Terminal mode (Discreet Mode).
    /// If None, dynamically calculated based on terminal viewport height.
    pub agent_lines: Option<u16>,
    pub immersive_mode: bool,
    pub page_overlap: bool,
    pub natural_breaks: bool,
    pub library_dirs: Vec<PathBuf>,
    /// Bookshelf directory. Files added via `import` (or its `add` alias) are copied into
    /// this directory; the original is left in place. Each book gets a
    /// sibling `*.json` sidecar. `reader shelf refresh` reconciles the
    /// directory contents, sidecars, and `library.json`.
    pub bookshelf_dir: Option<PathBuf>,
    /// Optional Calibre-Web / Calibre-Web Automated OPDS connection.
    pub network_library: NetworkLibraryConfig,
    pub reading_theme: crate::ui::ReadingTheme,
    pub agent_workspace: Option<String>,
    pub agent_task: Option<String>,
    pub agent_target: Option<String>,
    pub auto_reading_cpm: Option<u32>,
    pub auto_reading_mode: crate::auto_reading::AutoReadingMode,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            show_progress: true,
            discreet_mode: false,
            mouse_support: false,
            text_width: None,
            agent_lines: None,
            immersive_mode: false,
            page_overlap: true,
            natural_breaks: true,
            library_dirs: Vec::new(),
            bookshelf_dir: None,
            network_library: NetworkLibraryConfig::default(),
            reading_theme: crate::ui::ReadingTheme::Terminal,
            agent_workspace: None,
            agent_task: None,
            agent_target: None,
            auto_reading_cpm: None,
            auto_reading_mode: crate::auto_reading::AutoReadingMode::default(),
            path: None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NetworkLibraryConfig {
    pub enabled: bool,
    pub url: Option<String>,
    pub username: Option<String>,
    /// Explicit local credential file, so an installed executable works from any directory.
    pub env_file: Option<PathBuf>,
    /// Additional trusted PEM CA certificate for a private HTTPS server.
    pub ca_certificate: Option<PathBuf>,
}

impl ReaderConfig {
    pub fn open_default() -> Result<Self> {
        // Resolution strategy (XDG-first, with fallback) lives in `paths`.
        Self::open(crate::paths::configuration_path()?)
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        let mut config = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            match serde_json::from_str(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    // JSON is unparseable. Move the bad file aside and start
                    // fresh so the user is not stuck with a binary that
                    // refuses to launch. The original is preserved at
                    // `<path>.corrupt` for inspection.
                    eprintln!(
                        "warning: {} is corrupt ({err}); initializing a fresh configuration. \
                         The old file was preserved as {}.corrupt.",
                        path.display(),
                        path.display(),
                    );
                    let backup = path.with_extension("json.corrupt");
                    let _ = std::fs::rename(&path, &backup);
                    Self::default()
                }
            }
        } else {
            Self::default()
        };
        config.path = Some(path);
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(self)?;
        crate::atomic_file::write(path, text.as_bytes())
            .with_context(|| format!("Failed to save {}", path.display()))?;
        Ok(())
    }

    pub fn toggle_progress(&mut self) {
        self.show_progress = !self.show_progress;
    }

    pub fn toggle_discreet_mode(&mut self) {
        self.discreet_mode = !self.discreet_mode;
    }

    pub fn toggle_immersive_mode(&mut self) {
        self.immersive_mode = !self.immersive_mode;
    }

    pub fn toggle_page_overlap(&mut self) {
        self.page_overlap = !self.page_overlap;
    }

    pub fn toggle_natural_breaks(&mut self) {
        self.natural_breaks = !self.natural_breaks;
    }

    pub fn cycle_reading_theme(&mut self) -> crate::ui::ReadingTheme {
        self.reading_theme = self.reading_theme.next();
        self.reading_theme
    }

    pub fn set_reading_theme(&mut self, theme: crate::ui::ReadingTheme) {
        self.reading_theme = theme;
    }

    pub fn agent_workspace(&self) -> &str {
        self.agent_workspace
            .as_deref()
            .unwrap_or("data-mesh-service")
    }

    pub fn agent_task(&self) -> &str {
        self.agent_task
            .as_deref()
            .unwrap_or("Code Review & Refactor")
    }

    pub fn agent_target(&self) -> &str {
        self.agent_target
            .as_deref()
            .unwrap_or("crates/core/src/pipeline.rs")
    }

    pub fn auto_reading_cpm(&self) -> u32 {
        self.auto_reading_cpm.unwrap_or(300)
    }

    pub fn set_auto_reading_cpm(&mut self, cpm: u32) {
        self.auto_reading_cpm = Some(cpm.clamp(60, 2000));
    }

    pub fn auto_reading_mode(&self) -> crate::auto_reading::AutoReadingMode {
        self.auto_reading_mode
    }

    pub fn set_auto_reading_mode(&mut self, mode: crate::auto_reading::AutoReadingMode) {
        self.auto_reading_mode = mode;
    }
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct ReaderConfigWire {
    show_progress: bool,
    discreet_mode: Option<bool>,
    privacy_mode: Option<bool>,
    mouse_support: bool,
    text_width: Option<u16>,
    agent_lines: Option<u16>,
    immersive_mode: bool,
    page_overlap: Option<bool>,
    natural_breaks: Option<bool>,
    library_dirs: Vec<PathBuf>,
    bookshelf_dir: Option<PathBuf>,
    network_library: NetworkLibraryConfig,
    reading_theme: Option<crate::ui::ReadingTheme>,
    agent_workspace: Option<String>,
    agent_task: Option<String>,
    agent_target: Option<String>,
    auto_reading_cpm: Option<u32>,
    auto_reading_mode: Option<crate::auto_reading::AutoReadingMode>,
}

impl Default for ReaderConfigWire {
    fn default() -> Self {
        let config = ReaderConfig::default();
        Self {
            show_progress: config.show_progress,
            discreet_mode: None,
            privacy_mode: None,
            mouse_support: config.mouse_support,
            text_width: config.text_width,
            agent_lines: config.agent_lines,
            immersive_mode: config.immersive_mode,
            page_overlap: Some(config.page_overlap),
            natural_breaks: Some(config.natural_breaks),
            library_dirs: config.library_dirs,
            bookshelf_dir: config.bookshelf_dir,
            network_library: config.network_library,
            reading_theme: Some(config.reading_theme),
            agent_workspace: config.agent_workspace,
            agent_task: config.agent_task,
            agent_target: config.agent_target,
            auto_reading_cpm: config.auto_reading_cpm,
            auto_reading_mode: Some(config.auto_reading_mode),
        }
    }
}

impl<'de> serde::Deserialize<'de> for ReaderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ReaderConfigWire::deserialize(deserializer)?;
        Ok(Self {
            show_progress: wire.show_progress,
            discreet_mode: wire.discreet_mode.or(wire.privacy_mode).unwrap_or(false),
            mouse_support: wire.mouse_support,
            text_width: wire.text_width,
            agent_lines: wire.agent_lines,
            immersive_mode: wire.immersive_mode,
            page_overlap: wire.page_overlap.unwrap_or(true),
            natural_breaks: wire.natural_breaks.unwrap_or(true),
            library_dirs: wire.library_dirs,
            bookshelf_dir: wire.bookshelf_dir,
            network_library: wire.network_library,
            reading_theme: wire.reading_theme.unwrap_or_default(),
            agent_workspace: wire.agent_workspace,
            agent_task: wire.agent_task,
            agent_target: wire.agent_target,
            auto_reading_cpm: wire.auto_reading_cpm,
            auto_reading_mode: wire.auto_reading_mode.unwrap_or_default(),
            path: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_reading_friendly_display_settings() {
        let config = ReaderConfig::default();

        assert!(config.show_progress);
        assert!(!config.discreet_mode);
        assert!(!config.mouse_support);
        assert_eq!(config.text_width, None);
        assert!(config.page_overlap);
        assert!(config.natural_breaks);
        assert!(config.bookshelf_dir.is_none());
    }

    #[test]
    fn ignores_the_removed_theme_field_and_loads_missing_fields_from_defaults() {
        let config: ReaderConfig = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();

        assert!(config.show_progress);
        assert!(!config.discreet_mode);
        assert!(!config.mouse_support);
        assert_eq!(config.text_width, None);
        assert!(config.page_overlap);
        assert!(config.natural_breaks);
    }

    #[test]
    fn saves_and_reloads_toggles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("configuration.json");
        let mut config = ReaderConfig::open(path.clone()).unwrap();

        config.toggle_progress();
        config.toggle_discreet_mode();
        config.save().unwrap();

        let loaded = ReaderConfig::open(path).unwrap();
        assert!(!loaded.show_progress);
        assert!(loaded.discreet_mode);
    }

    #[test]
    fn recovers_from_corrupt_configuration_json_by_initializing_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("configuration.json");
        std::fs::write(&path, "not even close to json").unwrap();

        let config = ReaderConfig::open(path.clone()).unwrap();

        // The bad file was moved aside; the original path is gone.
        assert!(!path.exists());
        let backup = path.with_extension("json.corrupt");
        assert!(backup.exists());
        // Defaults are restored.
        assert!(config.show_progress);
        assert!(!config.discreet_mode);
        // And a subsequent save produces a valid file at the original path.
        config.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn migrates_legacy_privacy_mode_and_serializes_only_discreet_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("configuration.json");
        std::fs::write(&path, r#"{"privacy_mode":true}"#).unwrap();
        let config = ReaderConfig::open(path.clone()).unwrap();

        assert!(config.discreet_mode);
        config.save().unwrap();
        let serialized = std::fs::read_to_string(path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(value["discreet_mode"], true);
        assert!(value.get("privacy_mode").is_none());
    }

    #[test]
    fn discreet_mode_wins_when_both_config_keys_exist() {
        let config: ReaderConfig =
            serde_json::from_str(r#"{"privacy_mode":true,"discreet_mode":false}"#).unwrap();

        assert!(!config.discreet_mode);
    }

    #[test]
    fn removed_theme_is_not_serialized() {
        let config: ReaderConfig = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        let serialized = serde_json::to_value(config).unwrap();

        assert!(serialized.get("theme").is_none());
    }

    #[test]
    fn network_settings_round_trip_without_a_password_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("configuration.json");
        let mut config = ReaderConfig::open(path.clone()).unwrap();
        config.network_library.enabled = true;
        config.network_library.url = Some("https://books.example/opds".into());
        config.network_library.username = Some("reader".into());
        config.network_library.env_file = Some(PathBuf::from("connection.env"));
        config.network_library.ca_certificate = Some(PathBuf::from("root.pem"));
        config.save().unwrap();
        let loaded = ReaderConfig::open(path.clone()).unwrap();
        assert!(loaded.network_library.enabled);
        assert_eq!(loaded.network_library.url, config.network_library.url);
        assert_eq!(
            loaded.network_library.username,
            config.network_library.username
        );
        assert_eq!(
            loaded.network_library.env_file,
            config.network_library.env_file
        );
        assert_eq!(
            loaded.network_library.ca_certificate,
            config.network_library.ca_certificate
        );
        assert!(!std::fs::read_to_string(path).unwrap().contains("password"));
        let legacy: ReaderConfig = serde_json::from_str("{}").unwrap();
        assert!(!legacy.network_library.enabled);
        assert!(legacy.network_library.url.is_none());
    }

    #[test]
    fn immersive_mode_defaults_to_false_and_round_trips() {
        let legacy: ReaderConfig = serde_json::from_str("{}").unwrap();
        assert!(!legacy.immersive_mode);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("configuration.json");
        let mut config = ReaderConfig::open(path.clone()).unwrap();
        assert!(!config.immersive_mode);
        config.toggle_immersive_mode();
        assert!(config.immersive_mode);
        config.save().unwrap();

        let loaded = ReaderConfig::open(path).unwrap();
        assert!(loaded.immersive_mode);
    }

    #[test]
    fn agent_workspace_and_task_settings_round_trip() {
        let legacy: ReaderConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.agent_workspace(), "data-mesh-service");
        assert_eq!(legacy.agent_task(), "Code Review & Refactor");
        assert_eq!(legacy.agent_target(), "crates/core/src/pipeline.rs");

        let json = r#"{"agent_workspace":"telemetry-hub","agent_task":"Stream Pipeline Audit","agent_target":"crates/ingest/src/stream.rs"}"#;
        let config: ReaderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.agent_workspace(), "telemetry-hub");
        assert_eq!(config.agent_task(), "Stream Pipeline Audit");
        assert_eq!(config.agent_target(), "crates/ingest/src/stream.rs");

        let serialized = serde_json::to_string(&config).unwrap();
        let reloaded: ReaderConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reloaded.agent_workspace(), "telemetry-hub");
        assert_eq!(reloaded.agent_task(), "Stream Pipeline Audit");
        assert_eq!(reloaded.agent_target(), "crates/ingest/src/stream.rs");
    }

    #[test]
    fn auto_reading_settings_default_and_round_trip() {
        let legacy: ReaderConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.auto_reading_cpm(), 300);
        assert_eq!(
            legacy.auto_reading_mode(),
            crate::auto_reading::AutoReadingMode::TimedPage
        );

        let json = r#"{"auto_reading_cpm":360,"auto_reading_mode":"line_scroll"}"#;
        let config: ReaderConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.auto_reading_cpm(), 360);
        assert_eq!(
            config.auto_reading_mode(),
            crate::auto_reading::AutoReadingMode::LineScroll
        );

        let serialized = serde_json::to_string(&config).unwrap();
        let reloaded: ReaderConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reloaded.auto_reading_cpm(), 360);
        assert_eq!(
            reloaded.auto_reading_mode(),
            crate::auto_reading::AutoReadingMode::LineScroll
        );
    }
}
