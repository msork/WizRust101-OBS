use std::{collections::HashMap, sync::OnceLock};

use eframe::egui::Color32;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SchoolColors {
    pub primary: String,
    pub secondary: String,
}

fn palette() -> &'static HashMap<String, SchoolColors> {
    static PALETTE: OnceLock<HashMap<String, SchoolColors>> = OnceLock::new();
    PALETTE.get_or_init(|| {
        serde_json::from_str(include_str!("../static/school-palette.json"))
            .expect("school palette JSON must be valid")
    })
}

pub fn colors(school: &str) -> Option<SchoolColors> {
    palette().get(school).cloned()
}

pub fn primary_color(school: &str) -> Color32 {
    colors(school)
        .map(|colors| parse_hex(&colors.primary))
        .unwrap_or(Color32::from_rgb(79, 73, 81))
}

fn parse_hex(value: &str) -> Color32 {
    let rgb = u32::from_str_radix(value.trim_start_matches('#'), 16)
        .expect("school color must be six digit hex");
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_has_the_requested_primary_and_secondary_colors() {
        let expected = [
            ("Fire", "#C93424", "#F28C28"),
            ("Ice", "#4B9CD3", "#DCECF4"),
            ("Storm", "#6D3FC0", "#B28BE8"),
            ("Myth", "#D4A514", "#315B9D"),
            ("Life", "#4D8B3A", "#8A9A3A"),
            ("Death", "#29252E", "#716578"),
            ("Balance", "#A66A32", "#D7B77A"),
        ];
        for (school, primary, secondary) in expected {
            assert_eq!(
                colors(school),
                Some(SchoolColors {
                    primary: primary.into(),
                    secondary: secondary.into(),
                })
            );
        }
    }

    #[test]
    fn primary_colors_convert_to_ui_color_values() {
        assert_eq!(primary_color("Storm"), Color32::from_rgb(0x6D, 0x3F, 0xC0));
        assert_eq!(primary_color("Myth"), Color32::from_rgb(0xD4, 0xA5, 0x14));
    }
}
