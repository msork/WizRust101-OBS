#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameEvent {
    ZoneChanged { raw_zone_id: String },
    CharacterSelection,
}

#[derive(Default)]
pub struct LogParser;

impl LogParser {
    /// Parses only the exact zone and character-list markers established by
    /// WizRust101-RPC. Other records are deliberately ignored.
    pub fn parse_line(&mut self, line: &str) -> Vec<GameEvent> {
        if line.contains("CHARACTER LIST") {
            return vec![GameEvent::CharacterSelection];
        }
        parse_zone_id(line)
            .map(|raw_zone_id| vec![GameEvent::ZoneChanged { raw_zone_id }])
            .unwrap_or_default()
    }
}

fn parse_zone_id(line: &str) -> Option<String> {
    let marker = "zone = ";
    let start = line.find(marker)? + marker.len();
    let (value, _) = line[start..].split_once(',')?;
    let value = value.trim();
    (!value.is_empty() && !value.chars().any(char::is_control)).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_zone_and_character_list() {
        let mut parser = LogParser;
        assert_eq!(
            parser.parse_line("[log] zone = WizardCity/WC_Ravenwood, extra=1"),
            vec![GameEvent::ZoneChanged {
                raw_zone_id: "WizardCity/WC_Ravenwood".into()
            }]
        );
        assert_eq!(
            parser.parse_line("CHARACTER LIST"),
            vec![GameEvent::CharacterSelection]
        );
    }

    #[test]
    fn malformed_or_unrelated_lines_are_ignored() {
        let mut parser = LogParser;
        for line in [
            "zone = ,",
            "zone = missing comma",
            "Updating health globe (new health: 5, new health max: 10)",
            "quest objective: talk to someone",
        ] {
            assert!(parser.parse_line(line).is_empty(), "{line}");
        }
    }
}
