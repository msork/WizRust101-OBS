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
    pub world_icon: String,
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
    #[serde(default)]
    pub world_icon: String,
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
    party_status: Arc<Mutex<Option<String>>>,
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
            party_status: Arc::new(Mutex::new(None)),
        }
    }
    pub fn snapshot(&self) -> OverlayState {
        self.enrich(self.state.lock().unwrap().clone())
    }
    fn enrich(&self, mut snapshot: OverlayState) -> OverlayState {
        snapshot.world_icon = crate::presentation::world_icon(snapshot.world.as_deref()).into();
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
                world_icon: snapshot.world_icon.clone(),
                zone: snapshot.zone.clone(),
                session_seconds: snapshot.session_seconds,
            });
        snapshot.party = self
            .party
            .lock()
            .unwrap()
            .values()
            .cloned()
            .map(|mut member| {
                member.world_icon = crate::presentation::world_icon(member.world.as_deref()).into();
                member
            })
            .collect();
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
    pub fn set_peer_presence(&self, presence: WizardPresence) -> bool {
        if !crate::peer::valid_presence(&presence) {
            return false;
        }
        let mut party = self.party.lock().unwrap();
        if !party.contains_key(&presence.peer_id) && party.len() >= crate::peer::MAX_GUESTS {
            return false;
        }
        party.insert(presence.peer_id.clone(), presence);
        drop(party);
        self.publish_current();
        true
    }
    pub fn remove_peer(&self, peer_id: &str) {
        if self.party.lock().unwrap().remove(peer_id).is_some() {
            self.publish_current();
        }
    }
    pub fn replace_party(&self, members: Vec<WizardPresence>, local_member_id: &str) -> bool {
        if members.len() > crate::peer::MAX_PARTY_SIZE
            || members
                .iter()
                .any(|member| !crate::peer::valid_presence(member))
        {
            return false;
        }
        let mut ids = std::collections::HashSet::new();
        if members.iter().any(|member| {
            member.peer_id.is_empty()
                || member.peer_id.len() > 64
                || !ids.insert(member.peer_id.as_str())
        }) {
            return false;
        }
        let roster = members
            .into_iter()
            .filter(|member| member.peer_id != local_member_id)
            .map(|member| (member.peer_id.clone(), member))
            .collect::<BTreeMap<_, _>>();
        if roster.len() > crate::peer::MAX_GUESTS {
            return false;
        }
        *self.party.lock().unwrap() = roster;
        self.publish_current();
        true
    }
    pub fn clear_party(&self) {
        let changed = {
            let mut party = self.party.lock().unwrap();
            if party.is_empty() {
                false
            } else {
                party.clear();
                true
            }
        };
        if changed {
            self.publish_current();
        }
    }
    pub fn set_party_status(&self, status: Option<String>) {
        *self.party_status.lock().unwrap() = status;
    }
    pub fn party_status(&self) -> Option<String> {
        self.party_status.lock().unwrap().clone()
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
    fn mock_location_controls_publish_normal_live_state_updates() {
        let state = SharedState::new(AppConfig::default());
        let mut updates = state.subscribe();
        state.set_demo_state("Wizard City", "The Commons", "mock/Wizard City/The Commons");
        let first = updates.try_recv().unwrap();
        assert_eq!(first.world.as_deref(), Some("Wizard City"));
        assert_eq!(first.zone.as_deref(), Some("The Commons"));
        assert!(first.active);

        state.set_demo_state("Krokotopia", "The Oasis", "mock/Krokotopia/The Oasis");
        let changed = updates.try_recv().unwrap();
        assert_eq!(changed.world.as_deref(), Some("Krokotopia"));
        assert_eq!(changed.zone.as_deref(), Some("The Oasis"));
        state.stop();
        assert!(!updates.try_recv().unwrap().active);
    }

    #[test]
    fn party_roster_replaces_remote_members_without_replacing_local_primary() {
        use crate::config::CharacterProfile;
        let mut config = AppConfig::default();
        config.profiles.push(CharacterProfile {
            id: "local-profile".into(),
            name: "Local Wizard".into(),
            school: "Life".into(),
            ..Default::default()
        });
        config.active_profile = Some("local-profile".into());
        let state = SharedState::new(config);
        let mut remote = WizardPresence {
            peer_id: "host".into(),
            name: "Remote Host".into(),
            school: "Storm".into(),
            active: true,
            world: Some("Wizard City".into()),
            world_icon: String::new(),
            zone: Some("The Commons".into()),
            session_seconds: 22,
        };
        assert!(state.replace_party(vec![remote.clone()], "member-self"));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.wizard.as_ref().unwrap().name, "Local Wizard");
        assert_eq!(snapshot.party.len(), 1);
        assert_eq!(snapshot.party[0].name, "Remote Host");

        remote.peer_id = "member-self".into();
        assert!(state.replace_party(vec![remote], "member-self"));
        assert!(state.snapshot().party.is_empty());
        assert_eq!(
            state.snapshot().wizard.as_ref().unwrap().name,
            "Local Wizard"
        );
    }

    #[test]
    fn malformed_and_duplicate_party_roster_entries_are_rejected() {
        let state = SharedState::new(AppConfig::default());
        let guest = WizardPresence {
            peer_id: "same".into(),
            name: "Guest".into(),
            school: "Myth".into(),
            ..Default::default()
        };
        assert!(!state.replace_party(vec![guest.clone(), guest], "local"));
        assert!(state.snapshot().party.is_empty());
    }

    #[test]
    fn party_state_caps_remote_wizards_and_allows_a_replacement_after_leave() {
        let state = SharedState::new(AppConfig::default());
        let presence = |peer_id: &str| WizardPresence {
            peer_id: peer_id.into(),
            name: format!("Wizard {peer_id}"),
            school: "Life".into(),
            active: true,
            ..Default::default()
        };
        for id in ["one", "two", "three"] {
            assert!(state.set_peer_presence(presence(id)));
        }
        assert_eq!(
            state.snapshot().party.len() + 1,
            crate::peer::MAX_PARTY_SIZE
        );

        let mut updated = presence("one");
        updated.zone = Some("New Zone".into());
        assert!(state.set_peer_presence(updated));
        assert_eq!(state.snapshot().party.len(), 3);
        assert!(!state.set_peer_presence(presence("four")));
        assert_eq!(state.snapshot().party.len(), 3);

        state.remove_peer("two");
        assert!(state.set_peer_presence(presence("four")));
        assert_eq!(state.snapshot().party.len(), 3);
        assert!(
            state
                .snapshot()
                .party
                .iter()
                .any(|member| member.peer_id == "four")
        );
    }

    #[test]
    fn party_snapshot_rejects_more_than_four_total_wizards() {
        let state = SharedState::new(AppConfig::default());
        let members = (0..5)
            .map(|index| WizardPresence {
                peer_id: format!("peer-{index}"),
                name: format!("Wizard {index}"),
                school: "Balance".into(),
                ..Default::default()
            })
            .collect();
        assert!(!state.replace_party(members, "local"));
        assert!(state.snapshot().party.is_empty());

        let valid_full_party = (0..crate::peer::MAX_PARTY_SIZE)
            .map(|index| WizardPresence {
                peer_id: if index == 0 {
                    "local".into()
                } else {
                    format!("peer-{index}")
                },
                name: format!("Wizard {index}"),
                school: "Balance".into(),
                ..Default::default()
            })
            .collect();
        assert!(state.replace_party(valid_full_party, "local"));
        assert_eq!(state.snapshot().party.len(), crate::peer::MAX_GUESTS);
    }

    #[test]
    fn party_snapshot_cannot_omit_local_member_and_smuggle_four_remote_wizards() {
        let state = SharedState::new(AppConfig::default());
        let members = (0..4)
            .map(|index| WizardPresence {
                peer_id: format!("remote-{index}"),
                name: format!("Wizard {index}"),
                school: "Balance".into(),
                ..Default::default()
            })
            .collect();
        assert!(!state.replace_party(members, "local"));
        assert!(state.snapshot().party.is_empty());
    }

    #[test]
    fn party_full_status_survives_roster_retries_until_explicitly_cleared() {
        let state = SharedState::new(AppConfig::default());
        state.set_party_status(Some("Party is full".into()));
        state.clear_party();
        assert_eq!(state.party_status().as_deref(), Some("Party is full"));
        state.set_party_status(None);
        assert_eq!(state.party_status(), None);
    }

    #[test]
    fn party_location_and_world_icon_updates_publish_without_reconnecting() {
        let state = SharedState::new(AppConfig::default());
        let mut updates = state.subscribe();
        let member = WizardPresence {
            peer_id: "guest".into(),
            name: "Guest".into(),
            school: "Storm".into(),
            active: true,
            world: Some("Wizard City".into()),
            zone: Some("The Commons".into()),
            ..Default::default()
        };
        assert!(state.replace_party(vec![member.clone()], "self"));
        let first = updates.try_recv().unwrap();
        assert_eq!(first.party[0].world_icon, "wizardcity");
        assert_eq!(first.party[0].zone.as_deref(), Some("The Commons"));

        let changed = WizardPresence {
            world: Some("Celestia".into()),
            zone: Some("Survey Camp".into()),
            ..member
        };
        assert!(state.replace_party(vec![changed], "self"));
        let second = updates.try_recv().unwrap();
        assert_eq!(second.party[0].world_icon, "celestia");
        assert_eq!(second.party[0].zone.as_deref(), Some("Survey Camp"));
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
