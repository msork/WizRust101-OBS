//! World asset lookup and display rules shared by local and party cards.

use serde::Deserialize;
use std::sync::OnceLock;

const WORLD_ASSETS: &str = include_str!("../static/world-assets.json");
#[derive(Deserialize)]
struct AssetCatalog {
    fallback: String,
    worlds: std::collections::HashMap<String, String>,
}

fn catalog() -> &'static AssetCatalog {
    static CATALOG: OnceLock<AssetCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(WORLD_ASSETS).expect("valid RPC world asset map"))
}

/// Returns the RPC-provided asset key, or its Wizard101 fallback key.
pub fn world_icon(world: Option<&str>) -> &'static str {
    let assets = catalog();
    world
        .filter(|name| !name.is_empty() && *name != "Unknown")
        .and_then(|name| assets.worlds.get(name))
        .map(String::as_str)
        .unwrap_or(assets.fallback.as_str())
}

/// A database result of `Unknown` is not player-facing location information.
pub fn visible_world(world: Option<&str>) -> Option<&str> {
    world.filter(|name| {
        !name.is_empty() && *name != "Unknown" && catalog().worlds.contains_key(*name)
    })
}

pub fn visible_zone(zone: Option<&str>) -> Option<&str> {
    zone.filter(|name| !name.is_empty() && *name != "Unknown")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationDisplay<'a> {
    pub icon: &'static str,
    pub world: Option<&'a str>,
    pub zone: Option<&'a str>,
}

pub fn primary_location<'a>(world: Option<&'a str>, zone: Option<&'a str>) -> LocationDisplay<'a> {
    LocationDisplay {
        icon: world_icon(world),
        world: visible_world(world),
        zone: visible_zone(zone),
    }
}

pub fn party_location<'a>(world: Option<&'a str>, zone: Option<&'a str>) -> LocationDisplay<'a> {
    LocationDisplay {
        icon: world_icon(world),
        world: None,
        zone: visible_zone(zone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_and_party_location_formatting_keep_local_world_name_only() {
        let local = primary_location(Some("Wizard City"), Some("The Commons"));
        assert_eq!(local.world, Some("Wizard City"));
        assert_eq!(local.zone, Some("The Commons"));
        assert_eq!(local.icon, "wizardcity");

        let party = party_location(Some("Wizard City"), Some("The Commons"));
        assert_eq!(party.world, None);
        assert_eq!(party.zone, Some("The Commons"));
        assert_eq!(party.icon, "wizardcity");
    }

    #[test]
    fn unknown_or_unmapped_world_uses_rpc_fallback_without_unknown_text() {
        for world in [None, Some("Unknown"), Some("Unmapped World")] {
            let local = primary_location(world, None);
            let party = party_location(world, None);
            assert_eq!(local.icon, "wizard101");
            assert_eq!(local.world, None);
            assert_eq!(local.zone, None);
            assert_eq!(party.icon, "wizard101");
            assert_eq!(party.world, None);
            assert_eq!(party.zone, None);
        }
    }

    #[test]
    fn known_world_without_location_keeps_world_but_omits_zone() {
        let local = primary_location(Some("Celestia"), Some("Unknown"));
        let party = party_location(Some("Celestia"), Some("Unknown"));
        assert_eq!(local.icon, "celestia");
        assert_eq!(local.world, Some("Celestia"));
        assert_eq!(local.zone, None);
        assert_eq!(party.icon, "celestia");
        assert_eq!(party.world, None);
        assert_eq!(party.zone, None);
    }

    #[test]
    fn db_world_without_rpc_art_uses_fallback_and_hides_its_world_label() {
        let local = primary_location(Some("Kembaalung Village"), Some("Unknown"));
        assert_eq!(local.icon, catalog().fallback);
        assert_eq!(local.world, None);
        assert_eq!(local.zone, None);
    }

    #[test]
    fn rpc_asset_catalog_covers_db_worlds_and_served_files() {
        let assets = catalog();
        let db = crate::mapping::runtime_catalog().unwrap();
        let worlds = db
            .zones
            .values()
            .filter_map(|zone| zone.world.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(worlds.len() >= 25, "expected complete DB world coverage");
        for world in worlds {
            let key = world_icon(Some(world));
            if let Some(mapped) = assets.worlds.get(world) {
                assert_eq!(key, mapped);
            } else {
                assert_eq!(key, assets.fallback);
            }
        }
        assert_eq!(world_icon(Some("Unknown")), "wizard101");
    }
}
