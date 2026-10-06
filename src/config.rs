use std::{fs, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const SCHOOLS: [&str; 7] = ["Fire", "Ice", "Storm", "Myth", "Life", "Death", "Balance"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub ui_theme: UiTheme,
    pub profiles: Vec<CharacterProfile>,
    pub active_profile: Option<String>,
    pub overlay: OverlayConfig,
    /// Runtime-only: hosting always requires an explicit action after launch.
    #[serde(skip_serializing)]
    pub collaboration_server_enabled: bool,
    pub upnp_port_forward: bool,
    pub advertised_host: String,
    pub peer_port: u16,
    pub manual_address_override: bool,
    /// Runtime-only Party invitations and client credentials are never saved.
    #[serde(skip_serializing)]
    pub peer_links: Vec<PeerCredential>,
    #[serde(flatten)]
    pub future: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiTheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PeerCredential {
    pub peer_id: String,
    /// URL-safe base64-encoded 32-byte Noise PSK. Keep the config private.
    pub secret: String,
    pub connect_url: Option<String>,
    pub label: String,
    pub expires_at_unix: Option<u64>,
    /// Set only after this client has received an authenticated Party welcome.
    /// Imported invites that have never connected remain one-shot attempts.
    #[serde(default)]
    pub auto_reconnect: bool,
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
            ui_theme: UiTheme::Light,
            profiles: vec![],
            active_profile: None,
            overlay: OverlayConfig::default(),
            collaboration_server_enabled: false,
            upnp_port_forward: false,
            advertised_host: String::new(),
            peer_port: crate::peer::PEER_PORT,
            manual_address_override: false,
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
            x_percent: 0.6,
            y_percent: 25.0,
            scale: 1.0,
            opacity: 1.0,
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
        if self.peer_links.len() > 256 {
            return Err("too many Party peer credentials are active in this app session".into());
        }
        if !(1024..=65535).contains(&self.peer_port) {
            return Err("party port must be between 1024 and 65535".into());
        }
        let mut peer_ids = std::collections::HashSet::new();
        for peer in &self.peer_links {
            if peer.peer_id.is_empty()
                || peer.peer_id.len() > 64
                || peer.peer_id == crate::peer::HOST_MEMBER_ID
                || !peer_ids.insert(&peer.peer_id)
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
            if let Some(url) = &peer.connect_url {
                crate::peer::validate_peer_url(url, &peer.peer_id)?;
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
    pub fn load_from_path(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path)?;
        let legacy_value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let has_legacy_party_state = legacy_value.as_object().is_some_and(|object| {
            object.contains_key("peer_links") || object.contains_key("collaboration_server_enabled")
        });
        let mut config: Self = serde_json::from_value(legacy_value)?;
        // Migrate only complete, untouched previous defaults. A user-edited
        // field makes the saved overlay their own configuration, so preserve it.
        let has_legacy_overlay_position = is_untouched_old_overlay_default(&config.overlay);
        if has_legacy_overlay_position {
            config.overlay = OverlayConfig::default();
        }
        config.collaboration_server_enabled = false;
        config.peer_links.clear();
        config.validate()?;
        if has_legacy_party_state || has_legacy_overlay_position {
            config.save_to_path(path)?;
        }
        Ok(config)
    }
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        Self::load_from_path(&Self::path()?)
    }
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        self.validate()?;
        fs::create_dir_all(path.parent().ok_or("config path has no parent")?)?;
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save_to_path(&Self::path()?)
    }
}

fn is_untouched_old_overlay_default(overlay: &OverlayConfig) -> bool {
    let old_default = |x_percent, y_percent| OverlayConfig {
        x_percent,
        y_percent,
        opacity: 0.92,
        ..OverlayConfig::default()
    };
    // Both the original inset default and the previously shipped zero-origin
    // default are migrated. All other fields must still match untouched defaults.
    overlay == &old_default(0.8, 1.0) || overlay == &old_default(0.0, 0.0)
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
    fn old_config_defaults_to_persistent_light_theme() {
        let decoded: AppConfig = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(decoded.ui_theme, UiTheme::Light);
        let mut dark = decoded;
        dark.ui_theme = UiTheme::Dark;
        let encoded = serde_json::to_string(&dark).unwrap();
        let restored: AppConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.ui_theme, UiTheme::Dark);
    }

    #[test]
    fn default_overlay_stack_is_top_anchored_over_the_shop_area() {
        let overlay = OverlayConfig::default();
        assert_eq!(overlay.x_percent, 0.6);
        assert_eq!(overlay.y_percent, 25.0);
        assert_eq!(overlay.scale, 1.0);
        assert_eq!(overlay.opacity, 1.0);
        assert_eq!(overlay.transition_seconds, 4.0);
        assert!(overlay.character_location);
        assert!(overlay.zone_transition);
    }

    #[test]
    fn legacy_default_overlay_position_migrates_and_is_saved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let mut legacy = AppConfig::default();
        legacy.overlay.x_percent = 0.8;
        legacy.overlay.y_percent = 1.0;
        legacy.overlay.opacity = 0.92;
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let loaded = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.overlay, OverlayConfig::default());
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["overlay"]["x_percent"], 0.6);
        assert_eq!(saved["overlay"]["y_percent"], 25.0);
    }

    #[test]
    fn prior_zero_origin_default_migrates_to_requested_overlay_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let mut previous = AppConfig::default();
        previous.overlay.x_percent = 0.0;
        previous.overlay.y_percent = 0.0;
        previous.overlay.opacity = 0.92;
        fs::write(&path, serde_json::to_vec_pretty(&previous).unwrap()).unwrap();

        let loaded = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.overlay, OverlayConfig::default());
    }

    #[test]
    fn custom_overlay_position_is_preserved_during_load() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let mut config = AppConfig::default();
        config.overlay.x_percent = 12.5;
        config.overlay.y_percent = 4.25;
        fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

        let loaded = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.overlay.x_percent, 12.5);
        assert_eq!(loaded.overlay.y_percent, 4.25);
    }

    #[test]
    fn customized_overlay_settings_do_not_trigger_old_default_migration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let mut config = AppConfig::default();
        config.overlay.x_percent = 0.0;
        config.overlay.y_percent = 0.0;
        config.overlay.opacity = 0.7;
        fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

        let loaded = AppConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.overlay.x_percent, 0.0);
        assert_eq!(loaded.overlay.y_percent, 0.0);
        assert_eq!(loaded.overlay.opacity, 0.7);
    }
    #[test]
    fn old_peer_credentials_do_not_gain_reconnect_permission() {
        let credential: PeerCredential = serde_json::from_str(
            r#"{"peer_id":"peer-a","secret":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","connect_url":null,"label":"","expires_at_unix":null}"#,
        )
        .unwrap();
        assert!(!credential.auto_reconnect);
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

    #[test]
    fn party_credentials_are_runtime_only_and_old_saved_party_state_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        let mut config = AppConfig::default();
        config.profiles.push(CharacterProfile {
            id: "saved-wizard".into(),
            name: "Persistent Wizard".into(),
            school: "Ice".into(),
            ..Default::default()
        });
        config.active_profile = Some("saved-wizard".into());
        config.collaboration_server_enabled = true;
        config.peer_links.push(PeerCredential {
            peer_id: "old-member".into(),
            secret: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
            connect_url: Some("ws://127.0.0.1:17842/peer?peer_id=old-member".into()),
            label: "Old Guest".into(),
            expires_at_unix: Some(u64::MAX),
            auto_reconnect: true,
        });

        config.save_to_path(&path).unwrap();
        let saved_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved_json.get("peer_links").is_none());
        assert!(saved_json.get("collaboration_server_enabled").is_none());

        let mut legacy = saved_json;
        legacy["collaboration_server_enabled"] = true.into();
        legacy["peer_links"] = serde_json::json!([{
            "peer_id": "old-member",
            "secret": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "connect_url": "ws://127.0.0.1:17842/peer?peer_id=old-member",
            "label": "Old Guest",
            "expires_at_unix": u64::MAX,
            "auto_reconnect": true
        }]);
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let loaded = AppConfig::load_from_path(&path).unwrap();
        assert!(!loaded.collaboration_server_enabled);
        assert!(loaded.peer_links.is_empty());
        assert_eq!(loaded.profiles[0].name, "Persistent Wizard");
        let migrated: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(migrated.get("peer_links").is_none());
        assert!(migrated.get("collaboration_server_enabled").is_none());
    }

    #[test]
    fn separate_config_paths_keep_mock_instances_independent() {
        let temp = tempfile::tempdir().unwrap();
        let a_path = temp.path().join("instance-a").join("config.json");
        let b_path = temp.path().join("instance-b").join("config.json");
        let mut a = AppConfig::default();
        a.profiles.push(CharacterProfile {
            id: "a".into(),
            name: "Wizard A".into(),
            school: "Life".into(),
            ..Default::default()
        });
        a.active_profile = Some("a".into());
        a.save_to_path(&a_path).unwrap();
        AppConfig::default().save_to_path(&b_path).unwrap();

        assert_eq!(
            AppConfig::load_from_path(&a_path).unwrap().profiles[0].name,
            "Wizard A"
        );
        assert!(
            AppConfig::load_from_path(&b_path)
                .unwrap()
                .profiles
                .is_empty()
        );
    }
}
