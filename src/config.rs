use std::{fs, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const SCHOOLS: [&str; 7] = ["Fire", "Ice", "Storm", "Myth", "Life", "Death", "Balance"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub profiles: Vec<CharacterProfile>,
    pub active_profile: Option<String>,
    pub overlay: OverlayConfig,
    pub collaboration_server_enabled: bool,
    pub upnp_port_forward: bool,
    pub advertised_host: String,
    pub peer_links: Vec<PeerCredential>,
    #[serde(flatten)]
    pub future: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PeerCredential {
    pub peer_id: String,
    /// URL-safe base64-encoded 32-byte Noise PSK. Keep the config private.
    pub secret: String,
    pub connect_url: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CharacterProfile {
    pub id: String,
    pub name: String,
    pub school: String,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OverlayConfig {
    pub character_location: bool,
    pub zone_transition: bool,
    pub x_percent: f32,
    pub y_percent: f32,
    pub scale: f32,
    pub opacity: f32,
    pub transition_seconds: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            profiles: vec![],
            active_profile: None,
            overlay: OverlayConfig::default(),
            collaboration_server_enabled: false,
            upnp_port_forward: false,
            advertised_host: String::new(),
            peer_links: vec![],
            future: Default::default(),
        }
    }
}
impl Default for CharacterProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            school: "Balance".into(),
            metadata: Default::default(),
        }
    }
}
impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            character_location: true,
            zone_transition: true,
            x_percent: 4.0,
            y_percent: 87.0,
            scale: 1.0,
            opacity: 0.92,
            transition_seconds: 4.0,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported schema_version".into());
        }
        for profile in &self.profiles {
            if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
                return Err("profile id and name are required".into());
            }
            if !SCHOOLS.contains(&profile.school.as_str()) {
                return Err(format!("invalid school: {}", profile.school));
            }
        }
        if self
            .active_profile
            .as_ref()
            .is_some_and(|id| !self.profiles.iter().any(|p| &p.id == id))
        {
            return Err("active_profile must identify a saved profile".into());
        }
        if self.peer_links.len() > 8 {
            return Err("at most eight paired peers are supported".into());
        }
        let mut peer_ids = std::collections::HashSet::new();
        for peer in &self.peer_links {
            if peer.peer_id.is_empty() || peer.peer_id.len() > 64 || !peer_ids.insert(&peer.peer_id)
            {
                return Err("peer IDs must be unique and 1–64 characters".into());
            }
            let secret = base64::Engine::decode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                &peer.secret,
            )
            .map_err(|_| "peer secret is invalid base64")?;
            if secret.len() != 32 {
                return Err("peer secrets must contain 32 bytes".into());
            }
            if peer
                .connect_url
                .as_ref()
                .is_some_and(|url| !url.starts_with("ws://"))
            {
                return Err("peer URL must use ws:// (Noise encrypts peer messages)".into());
            }
        }
        let o = &self.overlay;
        if !(0.0..=100.0).contains(&o.x_percent)
            || !(0.0..=100.0).contains(&o.y_percent)
            || !(0.25..=3.0).contains(&o.scale)
            || !(0.0..=1.0).contains(&o.opacity)
            || !(1.0..=20.0).contains(&o.transition_seconds)
        {
            return Err("overlay settings are outside supported ranges".into());
        }
        Ok(())
    }
    pub fn path() -> Result<PathBuf, String> {
        ProjectDirs::from("com", "msork", "WizRust101-OBS")
            .map(|d| d.config_dir().join("config.json"))
            .ok_or("could not locate user config directory".into())
    }
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let config: Self = serde_json::from_slice(&fs::read(path)?)?;
        config.validate()?;
        Ok(config)
    }
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.validate()?;
        let path = Self::path()?;
        fs::create_dir_all(path.parent().ok_or("config path has no parent")?)?;
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_profiles_round_trip_with_future_metadata() {
        let mut config = AppConfig::default();
        let mut p = CharacterProfile {
            id: "one".into(),
            name: "Example Wizard".into(),
            school: "Life".into(),
            ..Default::default()
        };
        p.metadata.insert("title".into(), "Grandmaster".into());
        config.active_profile = Some(p.id.clone());
        config.profiles.push(p);
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: AppConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, config);
        decoded.validate().unwrap();
    }
    #[test]
    fn rejects_arbitrary_school_names() {
        let mut c = AppConfig::default();
        c.profiles.push(CharacterProfile {
            id: "x".into(),
            name: "X".into(),
            school: "Moon".into(),
            ..Default::default()
        });
        assert!(c.validate().unwrap_err().contains("invalid school"));
    }
    #[test]
    fn peer_listener_and_port_forwarding_are_opt_in() {
        let config = AppConfig::default();
        assert!(!config.collaboration_server_enabled);
        assert!(!config.upnp_port_forward);
    }
}
