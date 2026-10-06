use crate::{mapping::ZoneCatalog, parser::LogParser, state::SharedState};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub async fn follow(
    path: PathBuf,
    state: SharedState,
    catalog: Arc<ZoneCatalog>,
) -> std::io::Result<()> {
    let mut tail =
        crate::log_tailer::LogTailer::open(&path, crate::log_tailer::StartPosition::End)?;
    let mut parser = LogParser;
    crate::replay::replay_prefix(&path, tail.offset(), &mut parser, &state, &catalog)?;
    loop {
        match tail.poll() {
            Ok(lines) => {
                for line in lines {
                    for event in parser.parse_line(&line) {
                        state.apply(event, &catalog);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                state.stop();
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        if !path.exists() {
            state.stop();
            return Ok(());
        }
    }
}
pub async fn run_demo(state: SharedState, _catalog: Arc<ZoneCatalog>) {
    let samples = [
        ("Wizard City", "The Commons", "WizardCity/WC_Hub"),
        ("Wizard City", "Ravenwood", "WizardCity/WC_Ravenwood"),
        ("Krokotopia", "The Oasis", "Krokotopia/KT_Hub"),
        (
            "Dragonspyre",
            "The Atheneum",
            "DragonSpire/DS_A1_Knowledge/DS_A1Hub_Library",
        ),
    ];
    let mut n = 0;
    loop {
        let (w, z, r) = samples[n % samples.len()];
        state.set_demo_state(w, z, r);
        n += 1;
        tokio::time::sleep(Duration::from_secs(9)).await;
        if n % 4 == 0 {
            state.stop();
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
    }
}
