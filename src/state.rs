use crate::{config::AppConfig, mapping::ZoneCatalog, parser::GameEvent};
use serde::Serialize;
use std::collections::BTreeMap;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct OverlayState {
    pub active: bool,
    pub world: Option<String>,
    pub zone: Option<String>,
    pub raw_zone: Option<String>,
    pub session_seconds: u64,
    pub changed_at_ms: u64,
    pub revision: u64,
    pub wizard: Option<WizardPresence>,
    pub party: Vec<WizardPresence>,
    pub overlay: crate::config::OverlayConfig,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct WizardPresence {
    pub peer_id: String,
    pub name: String,
    pub school: String,
    pub active: bool,
    pub world: Option<String>,
    pub zone: Option<String>,
    pub session_seconds: u64,
}
#[derive(Clone)]
pub struct SharedState {
    pub config: Arc<Mutex<AppConfig>>,
    state: Arc<Mutex<OverlayState>>,
    tx: broadcast::Sender<OverlayState>,
    session_started: Arc<Mutex<Option<Instant>>>,
    party: Arc<Mutex<BTreeMap<String, WizardPresence>>>,
}
impl SharedState {
    pub fn new(config: AppConfig) -> Self {
        let (tx, _) = broadcast::channel(32);
        Self {
            config: Arc::new(Mutex::new(config)),
            state: Arc::new(Mutex::new(OverlayState::default())),
            tx,
            session_started: Arc::new(Mutex::new(None)),
            party: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    pub fn snapshot(&self) -> OverlayState {
        self.enrich(self.state.lock().unwrap().clone())
    }
    fn enrich(&self, mut snapshot: OverlayState) -> OverlayState {
        snapshot.session_seconds = self
            .session_started
            .lock()
            .unwrap()
            .map_or(0, |started| started.elapsed().as_secs());
        let config = self.config.lock().unwrap();
        snapshot.overlay = config.overlay.clone();
        snapshot.wizard = config
            .active_profile
            .as_ref()
            .and_then(|id| config.profiles.iter().find(|p| &p.id == id))
            .map(|p| WizardPresence {
                peer_id: "self".into(),
                name: p.name.clone(),
                school: p.school.clone(),
                active: snapshot.active,
                world: snapshot.world.clone(),
                zone: snapshot.zone.clone(),
                session_seconds: snapshot.session_seconds,
            });
        snapshot.party = self.party.lock().unwrap().values().cloned().collect();
        snapshot
    }
    pub fn subscribe(&self) -> broadcast::Receiver<OverlayState> {
        self.tx.subscribe()
    }
    pub fn publish_current(&self) {
        let mut snapshot = self.snapshot();
        snapshot.changed_at_ms = now_ms();
        snapshot.revision += 1;
        self.state.lock().unwrap().revision = snapshot.revision;
        let _ = self.tx.send(snapshot);
    }
    pub fn local_presence(&self) -> Option<WizardPresence> {
        self.snapshot().wizard
    }
    pub fn set_peer_presence(&self, presence: WizardPresence) {
        if !crate::peer::valid_presence(&presence) {
            return;
        }
        self.party
            .lock()
            .unwrap()
            .insert(presence.peer_id.clone(), presence);
        self.publish_current();
    }
    pub fn remove_peer(&self, peer_id: &str) {
        if self.party.lock().unwrap().remove(peer_id).is_some() {
            self.publish_current();
        }
    }
    pub fn apply(&self, event: GameEvent, catalog: &ZoneCatalog) {
        let mut s = self.state.lock().unwrap();
        match event {
            GameEvent::ZoneChanged { raw_zone_id } => {
                if s.raw_zone.as_deref() == Some(&raw_zone_id) {
                    return;
                }
                let mapping = catalog.resolve(&raw_zone_id);
                s.raw_zone = Some(raw_zone_id);
                s.zone = mapping.map(|m| m.location.clone());
                s.world = mapping.and_then(|m| m.world.clone());
                s.active = true;
                let mut start = self.session_started.lock().unwrap();
                start.get_or_insert_with(Instant::now);
            }
            GameEvent::CharacterSelection => {
                s.active = false;
                s.world = None;
                s.zone = None;
                s.raw_zone = None;
                *self.session_started.lock().unwrap() = None;
            }
        }
        s.session_seconds = self
            .session_started
            .lock()
            .unwrap()
            .map_or(0, |t| t.elapsed().as_secs());
        s.changed_at_ms = now_ms();
        s.revision += 1;
        let _ = self.tx.send(self.enrich(s.clone()));
    }
    pub fn set_demo_state(&self, world: &str, zone: &str, raw: &str) {
        let mut s = self.state.lock().unwrap();
        if s.raw_zone.as_deref() == Some(raw) {
            return;
        }
        s.active = true;
        s.world = Some(world.into());
        s.zone = Some(zone.into());
        s.raw_zone = Some(raw.into());
        let mut started = self.session_started.lock().unwrap();
        started.get_or_insert_with(Instant::now);
        s.session_seconds = started.map_or(0, |i| i.elapsed().as_secs());
        drop(started);
        s.changed_at_ms = now_ms();
        s.revision += 1;
        let _ = self.tx.send(self.enrich(s.clone()));
    }
    pub fn stop(&self) {
        let mut s = self.state.lock().unwrap();
        s.active = false;
        s.world = None;
        s.zone = None;
        s.raw_zone = None;
        s.session_seconds = 0;
        *self.session_started.lock().unwrap() = None;
        s.changed_at_ms = now_ms();
        s.revision += 1;
        let _ = self.tx.send(self.enrich(s.clone()));
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{ZoneCatalog, ZoneMapping};
    use std::time::Duration;
    #[test]
    fn duplicate_zone_is_ignored_and_changes_resolve_exactly() {
        let state = SharedState::new(AppConfig::default());
        let mut c = ZoneCatalog::default();
        c.zones.insert(
            "A/one".into(),
            ZoneMapping {
                location: "The Commons".into(),
                world: Some("Wizard City".into()),
            },
        );
        state.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "A/one".into(),
            },
            &c,
        );
        let rev = state.snapshot().revision;
        state.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "A/one".into(),
            },
            &c,
        );
        assert_eq!(state.snapshot().revision, rev);
        state.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "A/unknown".into(),
            },
            &c,
        );
        let s = state.snapshot();
        assert_eq!(s.zone, None);
        assert_eq!(s.world, None);
        assert!(s.active);
        let _ = Duration::from_secs(0);
    }

    #[test]
    fn demo_state_sets_sample_location_and_stop_clears_session() {
        let state = SharedState::new(AppConfig::default());
        state.set_demo_state("Wizard City", "The Commons", "WizardCity/WC_Hub");
        let active = state.snapshot();
        assert!(active.active);
        assert_eq!(active.world.as_deref(), Some("Wizard City"));
        assert_eq!(active.zone.as_deref(), Some("The Commons"));
        state.stop();
        let stopped = state.snapshot();
        assert!(!stopped.active);
        assert_eq!(stopped.session_seconds, 0);
        assert_eq!(stopped.zone, None);
    }

    #[test]
    fn session_clock_continues_across_zone_changes_and_resets_on_stop() {
        let state = SharedState::new(AppConfig::default());
        let catalog = ZoneCatalog::default();
        state.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "A/one".into(),
            },
            &catalog,
        );
        std::thread::sleep(Duration::from_millis(1050));
        state.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "A/two".into(),
            },
            &catalog,
        );
        assert!(state.snapshot().session_seconds >= 1);
        state.stop();
        assert_eq!(state.snapshot().session_seconds, 0);
    }
}
