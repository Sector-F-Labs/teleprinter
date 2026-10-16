//! Settings kept between runs in `~/.config/teleprinter/config.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub skin: Option<String>,
    /// "light" or "dark" while the skin follows the desktop; anything else follows it.
    pub light_dark: String,
    pub system_title_bar: bool,
    pub font_size: f32,
    /// Space between panes, and around them, in points; 0 has them touch.
    pub gap: f32,
    /// How tall a pane's title bar is, in points, down to what its buttons need.
    pub pane_title_height: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { skin: None, light_dark: "system".into(), system_title_bar: false, font_size: 14.0, gap: 0.0, pane_title_height: 28.0 }
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("teleprinter").join("config.toml"))
}

impl Config {
    pub fn load() -> Self {
        path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn theme(&self) -> egui::ThemePreference {
        match self.light_dark.as_str() {
            "light" => egui::ThemePreference::Light,
            "dark" => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        }
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
        if let Ok(text) = toml::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }
}
