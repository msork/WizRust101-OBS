//! Compact location presentation helpers shared by state serialization and UI.

/// Selects an original, bundled vector sigil from the exact world label resolved
/// by the WizRust101-DB catalog. Related worlds share a visual motif.
pub fn world_icon(world: Option<&str>) -> &'static str {
    match world {
        Some("Wizard City" | "Krokotopia" | "Marleybone" | "Wysteria") => "spiral",
        Some("Dragonspyre" | "Aquila" | "Darkmoor" | "Castle Darkmoor") => "flame",
        Some("Celestia" | "Polaris" | "Empyrea" | "Wallaru") => "star",
        Some("Grizzleheim" | "Zafaria" | "Azteca") => "leaf",
        Some("MooShu" | "Avalon" | "Arcanum") => "moon",
        Some("Khrysalis" | "Mirage" | "Novus" | "Lemuria" | "Karamelle") => "crown",
        Some("Kembaalung Village" | "Zigazag" | "PetDerby" | "Raids") => "mountain",
        Some(_) | None => "unknown",
    }
}

/// The primary wizard may show the resolved world name and current zone.
pub fn primary_location(world: Option<&str>, zone: Option<&str>) -> String {
    match (
        world.filter(|value| !value.is_empty()),
        zone.filter(|value| !value.is_empty()),
    ) {
        (Some(world), Some(zone)) => format!("{world} — {zone}"),
        (Some(world), None) => format!("{world} — Location unknown"),
        (None, Some(zone)) => format!("Unknown World — {zone}"),
        (None, None) => "Unknown World — Location unknown".into(),
    }
}

/// Party members keep their location compact: their resolved world is encoded
/// in the sigil, while only the player-facing zone is written as text.
pub fn party_location(zone: Option<&str>) -> &str {
    zone.filter(|value| !value.is_empty())
        .unwrap_or("Location unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_and_party_location_formatting_preserve_local_priority() {
        assert_eq!(
            primary_location(Some("Wizard City"), Some("The Commons")),
            "Wizard City — The Commons"
        );
        assert_eq!(party_location(Some("The Commons")), "The Commons");
        assert_eq!(party_location(None), "Location unknown");
    }

    #[test]
    fn world_sigil_uses_resolved_world_and_has_unknown_fallback() {
        assert_eq!(world_icon(Some("Wizard City")), "spiral");
        assert_eq!(world_icon(Some("Celestia")), "star");
        assert_eq!(world_icon(Some("Unknown")), "unknown");
        assert_eq!(world_icon(Some("unmapped future world")), "unknown");
        assert_eq!(world_icon(None), "unknown");
    }
}
