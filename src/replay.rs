use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::Path,
};

use crate::{mapping::ZoneCatalog, parser::LogParser, state::SharedState};

/// Replays complete records from the log prefix captured when the tailer opens.
/// The tailer starts at that byte limit so lines appended during replay remain
/// available to the live follower.
pub fn replay_prefix(
    path: impl AsRef<Path>,
    byte_limit: u64,
    parser: &mut LogParser,
    state: &SharedState,
    catalog: &ZoneCatalog,
) -> io::Result<usize> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file.take(byte_limit));
    let mut bytes = Vec::new();
    let mut records = 0;
    loop {
        bytes.clear();
        let n = reader.read_until(b'\n', &mut bytes)?;
        if n == 0 || bytes.last() != Some(&b'\n') {
            break;
        }
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let line = String::from_utf8_lossy(&bytes);
        for event in parser.parse_line(&line) {
            state.apply(event, catalog);
        }
        records += 1;
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, state::SharedState};
    use std::fs;
    use tempfile::tempdir;
    #[test]
    fn replays_last_zone_from_complete_prefix_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("WizardClient.log");
        let prefix = b"zone = World/Old,\nzone = World/New,\n";
        fs::write(&path, [prefix.as_slice(), b"zone = partial"].concat()).unwrap();
        let state = SharedState::new(AppConfig::default());
        let n = replay_prefix(
            &path,
            prefix.len() as u64,
            &mut LogParser,
            &state,
            &ZoneCatalog::default(),
        )
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(state.snapshot().raw_zone.as_deref(), Some("World/New"));
    }
}
