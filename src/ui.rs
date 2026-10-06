use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use crate::{
    config::{AppConfig, CharacterProfile, SCHOOLS},
    peer::{self},
    state::SharedState,
};
use eframe::egui::{
    self, Align, Color32, ComboBox, Context, Frame, Layout, RichText, Stroke, ViewportCommand,
};

const INK: Color32 = Color32::from_rgb(47, 34, 42);
const PAPER: Color32 = Color32::from_rgb(239, 225, 190);
const PAPER_LIGHT: Color32 = Color32::from_rgb(249, 240, 215);
const GOLD: Color32 = Color32::from_rgb(177, 132, 55);
const RED: Color32 = Color32::from_rgb(119, 48, 53);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overlay,
    Wizard,
    Party,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Open,
    #[cfg(target_os = "linux")]
    Quit,
}

pub fn run(
    shared: SharedState,
    config_path: PathBuf,
    http_port: u16,
    display_name: String,
    demo_mode: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let overlay_url = format!("http://127.0.0.1:{http_port}/overlay");
    let (icon_rgba, icon_width, icon_height) =
        decode_icon(include_bytes!("../assets/icons/sizes/256.png"))?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("WizRust101-OBS — {display_name}"))
            .with_inner_size([850.0, 690.0])
            .with_min_inner_size([700.0, 560.0])
            .with_visible(false)
            .with_icon(egui::IconData {
                rgba: icon_rgba,
                width: icon_width,
                height: icon_height,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "WizRust101-OBS",
        options,
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(
                shared.clone(),
                cc.egui_ctx.clone(),
                config_path.clone(),
                overlay_url.clone(),
                display_name.clone(),
                demo_mode,
            )))
        }),
    )?;
    Ok(())
}

fn install_theme(ctx: &Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = PAPER;
    v.window_fill = PAPER_LIGHT;
    v.extreme_bg_color = Color32::from_rgb(255, 249, 232);
    v.faint_bg_color = Color32::from_rgb(228, 211, 171);
    v.override_text_color = Some(INK);
    v.selection.bg_fill = Color32::from_rgb(196, 158, 87);
    v.widgets.noninteractive.bg_fill = PAPER;
    v.widgets.inactive.bg_fill = Color32::from_rgb(231, 216, 180);
    v.widgets.hovered.bg_fill = Color32::from_rgb(222, 199, 147);
    v.widgets.active.bg_fill = Color32::from_rgb(196, 158, 87);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, GOLD);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 9.0);
        s.visuals.window_corner_radius = egui::CornerRadius::same(13);
    });
}

struct SettingsApp {
    shared: SharedState,
    config_path: PathBuf,
    overlay_url: String,
    demo_mode: bool,
    demo_world: String,
    demo_zone: String,
    draft: AppConfig,
    tab: Tab,
    status: String,
    invite_text: String,
    import_text: String,
    selected_profile: String,
    quit: Arc<Mutex<bool>>,
    tray_rx: Receiver<TrayAction>,
    _tray: TrayLifetime,
}
enum TrayLifetime {
    #[cfg(target_os = "linux")]
    Linux(std::sync::mpsc::Sender<LinuxCommand>),
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    Native { _icon: tray_icon::TrayIcon },
}
#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum LinuxCommand {
    Stop,
}

impl SettingsApp {
    fn new(
        shared: SharedState,
        ctx: Context,
        config_path: PathBuf,
        overlay_url: String,
        display_name: String,
        demo_mode: bool,
    ) -> Self {
        let draft = shared.config.lock().unwrap().clone();
        let initial_state = shared.snapshot();
        let selected_profile = draft
            .active_profile
            .clone()
            .or_else(|| draft.profiles.first().map(|p| p.id.clone()))
            .unwrap_or_default();
        let quit = Arc::new(Mutex::new(false));
        let (tray_rx, tray) = install_tray(ctx, quit.clone(), display_name.clone());
        Self {
            shared,
            config_path,
            overlay_url,
            demo_mode,
            demo_world: initial_state.world.unwrap_or_else(|| "Wizard City".into()),
            demo_zone: initial_state.zone.unwrap_or_else(|| "The Commons".into()),
            draft,
            tab: Tab::Overlay,
            status: "Ready • your settings stay on this device".into(),
            invite_text: String::new(),
            import_text: String::new(),
            selected_profile,
            quit,
            tray_rx,
            _tray: tray,
        }
    }
    fn save(&mut self) {
        if !self.draft.collaboration_server_enabled {
            self.draft.upnp_port_forward = false;
        }
        match self.draft.save_to_path(&self.config_path) {
            Ok(()) => {
                *self.shared.config.lock().unwrap() = self.draft.clone();
                self.shared.publish_current();
                self.status = "Saved to your local configuration".into()
            }
            Err(e) => self.status = format!("Could not save settings: {e}"),
        }
    }
    fn profile_mut(&mut self) -> Option<&mut CharacterProfile> {
        self.draft
            .profiles
            .iter_mut()
            .find(|p| p.id == self.selected_profile)
    }
    fn create_invite(&mut self) {
        if !self.draft.collaboration_server_enabled {
            self.status = "Start hosting before creating an invite".into();
            return;
        }
        let connected: std::collections::HashSet<_> = self
            .shared
            .snapshot()
            .party
            .into_iter()
            .map(|member| member.peer_id)
            .collect();
        self.draft.peer_links.retain(|peer| {
            peer.expires_at_unix
                .is_none_or(|expires| expires > peer::unix_now())
                || connected.contains(&peer.peer_id)
        });
        if self.draft.peer_links.len() >= 8 {
            self.status = "This party has reached its eight invite limit".into();
            return;
        }
        let Some(profile) = self
            .draft
            .active_profile
            .as_ref()
            .and_then(|id| self.draft.profiles.iter().find(|p| &p.id == id))
        else {
            self.status = "Choose a primary wizard before hosting a party".into();
            return;
        };
        let host_label = profile.name.clone();
        let id = format!("member-{}", rand::random::<u64>());
        match peer::create_pairing(id) {
            Ok((mut credential, _)) => {
                credential.label = host_label;
                let host = if self.draft.manual_address_override {
                    self.draft.advertised_host.trim().to_owned()
                } else {
                    match peer::suggested_host(self.draft.upnp_port_forward) {
                        Ok(host) => host,
                        Err(_) if !self.draft.advertised_host.trim().is_empty() => {
                            self.draft.advertised_host.trim().to_owned()
                        }
                        Err(error) => {
                            self.status = format!("Could not prepare an invite: {error}");
                            return;
                        }
                    }
                };
                self.draft.advertised_host = host.clone();
                match peer::invite_for(&credential, &host, self.draft.peer_port) {
                    Ok(invite) => {
                        self.invite_text = peer::encode_invite(&invite).unwrap_or_default();
                        self.draft.peer_links.push(credential);
                        self.save();
                        self.status = "Party invite ready to copy".into();
                    }
                    Err(error) => self.status = format!("Could not prepare an invite: {error}"),
                }
            }
            Err(e) => self.status = e,
        }
    }
    fn import_invite(&mut self) {
        if self
            .draft
            .active_profile
            .as_ref()
            .and_then(|id| self.draft.profiles.iter().find(|profile| &profile.id == id))
            .is_none_or(|profile| profile.name.trim().is_empty())
        {
            self.status = "Choose a named primary wizard before joining a party".into();
            return;
        }
        match peer::import_invite(&self.import_text, String::new()) {
            Ok(link) => {
                self.draft.collaboration_server_enabled = false;
                self.draft.upnp_port_forward = false;
                self.draft.peer_links.clear();
                self.draft.peer_links.push(link);
                self.shared.clear_party();
                self.save();
                self.status = "Joining party. Waiting for the host…".into()
            }
            Err(e) => self.status = e,
        }
    }
    fn host_party(&mut self) {
        if self.draft.active_profile.is_none() {
            self.status = "Choose a primary wizard before hosting a party".into();
            return;
        }
        self.draft.peer_links.clear();
        self.shared.clear_party();
        self.draft.collaboration_server_enabled = true;
        self.create_invite();
    }
    fn leave_party(&mut self) {
        self.draft.collaboration_server_enabled = false;
        self.draft.upnp_port_forward = false;
        self.draft.peer_links.clear();
        self.invite_text.clear();
        self.shared.clear_party();
        self.save();
        self.status = "You left the party".into();
    }
    fn top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            sigil(ui);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("WIZRUST101 • OBS SPELLBOOK")
                        .size(22.0)
                        .strong()
                        .color(RED),
                );
                ui.label(
                    RichText::new("A quiet companion for your Wizard101 stream")
                        .size(13.0)
                        .color(Color32::from_rgb(102, 81, 59)),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if brass_button(ui, "Save changes").clicked() {
                    self.save();
                }
                if ui.button("Minimize to tray").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
                }
            })
        });
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Overlay, "Overlay"),
                (Tab::Wizard, "My Wizard"),
                (Tab::Party, "Party & Invitations"),
            ] {
                let selected = self.tab == tab;
                let response = ui.selectable_label(
                    selected,
                    RichText::new(label)
                        .strong()
                        .color(if selected { RED } else { INK }),
                );
                if response.clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.separator();
    }
    fn overlay_ui(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "THE STREAM CANVAS",
            "Keep the game in view; let the overlay sit lightly at the edge.",
        );
        ui.add_space(8.0);
        let o = &mut self.draft.overlay;
        setting_toggle(
            ui,
            "Character and location plaque",
            &mut o.character_location,
            "Your chosen wizard with the live world and zone.",
        );
        setting_toggle(
            ui,
            "Zone arrival flourish",
            &mut o.zone_transition,
            "A brief location reveal when a new zone is observed.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Position • X");
            ui.add(egui::Slider::new(&mut o.x_percent, 0.0..=100.0).suffix("%"));
            ui.label("Y");
            ui.add(egui::Slider::new(&mut o.y_percent, 0.0..=100.0).suffix("%"));
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            ui.add(egui::Slider::new(&mut o.scale, 0.25..=3.0));
            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut o.opacity, 0.0..=1.0));
        });
        ui.horizontal(|ui| {
            ui.label("Reveal duration");
            ui.add(egui::Slider::new(&mut o.transition_seconds, 1.0..=20.0).suffix(" sec"));
        });
        if self.demo_mode {
            ui.add_space(12.0);
            section(
                ui,
                "MOCK WIZARD LOCATION",
                "These controls update the normal local state and real Party network feed.",
            );
            ui.horizontal(|ui| {
                ui.label("World");
                ui.add(egui::TextEdit::singleline(&mut self.demo_world).desired_width(160.0));
                ui.label("Zone");
                ui.add(egui::TextEdit::singleline(&mut self.demo_zone).desired_width(190.0));
                if brass_button(ui, "Apply location").clicked() {
                    let raw = format!("mock/{}/{}", self.demo_world, self.demo_zone);
                    self.shared
                        .set_demo_state(&self.demo_world, &self.demo_zone, &raw);
                    self.status = "Mock wizard location updated".into();
                }
                if ui.button("End mock session").clicked() {
                    self.shared.stop();
                    self.status = "Mock wizard session ended".into();
                }
            });
        }
        ui.add_space(16.0);
        section(
            ui,
            "OBS BROWSER SOURCE",
            "Add this URL as a Browser Source • canvas 1920 x- 1080",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.overlay_url).monospace());
            if brass_button(ui, "Copy overlay URL").clicked() {
                ui.ctx().copy_text(self.overlay_url.clone());
                self.status = "OBS overlay URL copied".into();
            }
        });
    }
    fn wizard_ui(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "YOUR CHARACTER",
            "Identity is always supplied by you; the log does not reliably identify the selected wizard.",
        );
        ui.horizontal(|ui| {
            ui.label("Saved profile");
            ComboBox::from_id_salt("profiles")
                .selected_text(
                    self.draft
                        .profiles
                        .iter()
                        .find(|p| p.id == self.selected_profile)
                        .map(|p| p.name.as_str())
                        .unwrap_or("Choose or create a profile"),
                )
                .show_ui(ui, |ui| {
                    for p in &self.draft.profiles {
                        ui.selectable_value(&mut self.selected_profile, p.id.clone(), &p.name);
                    }
                });
            if brass_button(ui, "+ New wizard").clicked() {
                let id = format!("profile-{}", rand::random::<u32>());
                self.draft.profiles.push(CharacterProfile {
                    id: id.clone(),
                    name: String::new(),
                    school: "Balance".into(),
                    metadata: Default::default(),
                });
                self.selected_profile = id;
            }
        });
        if let Some(p) = self.profile_mut() {
            ui.add_space(12.0);
            ui.label("WIZARD NAME");
            ui.add(
                egui::TextEdit::singleline(&mut p.name)
                    .hint_text("Enter the character name shown on stream")
                    .desired_width(460.0),
            );
            ui.add_space(8.0);
            ui.label("SCHOOL");
            ComboBox::from_id_salt("school")
                .selected_text(&p.school)
                .show_ui(ui, |ui| {
                    for school in SCHOOLS {
                        ui.selectable_value(&mut p.school, school.to_string(), school);
                    }
                });
            ui.horizontal(|ui| {
                ui.label("School sigil");
                let color = school_color(&p.school);
                ui.painter().circle_filled(
                    ui.cursor().left_top() + egui::vec2(12.0, 12.0),
                    10.0,
                    color,
                );
                ui.add_space(28.0);
                ui.label(RichText::new(&p.school).strong().color(color));
            });
            if brass_button(ui, "Make primary wizard").clicked() {
                self.draft.active_profile = Some(self.selected_profile.clone());
                self.status = "Primary wizard selected".into();
            }
            if ui.button("Remove this profile").clicked() {
                self.draft
                    .profiles
                    .retain(|p| p.id != self.selected_profile);
                self.selected_profile = self
                    .draft
                    .profiles
                    .first()
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
                self.draft.active_profile = self.draft.profiles.first().map(|p| p.id.clone());
            }
        } else {
            ui.add_space(16.0);
            ui.label("Create a saved wizard profile to personalize the overlay.");
        }
    }
    fn party_ui(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "YOUR WIZARDS, TOGETHER",
            "Share a small live roster while every stream keeps its own wizard in front.",
        );
        ui.add_space(10.0);
        let hosting = self.draft.collaboration_server_enabled;
        let joining = self
            .draft
            .peer_links
            .iter()
            .any(|p| p.connect_url.is_some());
        let members = self.shared.snapshot().party;
        Frame::new()
            .fill(PAPER_LIGHT)
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(207, 180, 123)))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✧").size(24.0).color(GOLD));
                    ui.vertical(|ui| {
                        let invite_expired = self.draft.peer_links.iter().any(|peer| {
                            peer.connect_url.is_some()
                                && peer
                                    .expires_at_unix
                                    .is_some_and(|expires| expires <= peer::unix_now())
                        });
                        let (title, detail) = if hosting {
                            ("Hosting Party", "Your invite is ready to share")
                        } else if joining && members.is_empty() && invite_expired {
                            ("Invite Expired", "Paste a fresh party invite to reconnect")
                        } else if joining && members.is_empty() {
                            ("Joining Party", "Waiting for the host to return")
                        } else if joining {
                            ("In a Party", "Your wizard is connected")
                        } else {
                            ("No Party Yet", "Host a party or paste an invite to join")
                        };
                        ui.label(RichText::new(title).strong().color(RED));
                        ui.label(RichText::new(detail).small().color(INK));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if hosting {
                            if brass_button(ui, "Create Invite").clicked() {
                                self.create_invite();
                            }
                        } else if !joining && brass_button(ui, "Host Party").clicked() {
                            self.host_party();
                        }
                        if (hosting || joining) && ui.button("Leave Party").clicked() {
                            self.leave_party();
                        }
                    });
                });
                if hosting && !self.invite_text.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Invite ready - valid for 24 hours").small());
                        if brass_button(ui, "Copy Invite").clicked() {
                            ui.ctx().copy_text(self.invite_text.clone());
                            self.status = "Party invite copied".into();
                        }
                    });
                }
            });

        ui.add_space(12.0);
        ui.label(RichText::new("JOIN A PARTY").strong().color(RED));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.import_text)
                    .hint_text("Paste a party invite here")
                    .desired_width(580.0),
            );
            if brass_button(ui, "Join Party").clicked() {
                self.import_invite();
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("PARTY ROSTER").strong().color(RED));
            ui.label(
                RichText::new(format!("{} connected", members.len()))
                    .small()
                    .color(INK),
            );
        });
        if members.is_empty() {
            ui.label(
                RichText::new("Connected wizards will appear here.")
                    .small()
                    .color(INK),
            );
        } else {
            for member in &members {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").color(if member.active {
                        Color32::from_rgb(94, 135, 79)
                    } else {
                        GOLD
                    }));
                    ui.label(
                        RichText::new(&member.name)
                            .strong()
                            .color(school_color(&member.school)),
                    );
                    ui.label(RichText::new(format!("- {}", member.school)).small());
                    let status = if member.active {
                        format!(
                            "Online - {}",
                            [member.world.as_deref(), member.zone.as_deref()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" - ")
                        )
                    } else {
                        "Online - between worlds".into()
                    };
                    ui.label(RichText::new(status).small().color(INK));
                });
            }
        }
        if hosting {
            for invite in self
                .draft
                .peer_links
                .iter()
                .filter(|p| p.connect_url.is_none())
            {
                if !members
                    .iter()
                    .any(|member| member.peer_id == invite.peer_id)
                {
                    let expired = invite
                        .expires_at_unix
                        .is_some_and(|t| t <= crate::peer::unix_now());
                    ui.label(
                        RichText::new(if expired {
                            "Invite expired"
                        } else {
                            "Invite sent - waiting to join"
                        })
                        .small()
                        .color(Color32::from_rgb(110, 89, 67)),
                    );
                }
            }
        }

        ui.add_space(10.0);
        setting_toggle(
            ui,
            "Automatic router setup",
            &mut self.draft.upnp_port_forward,
            "Optional. Off by default; used only while hosting to help friends connect over the internet.",
        );
        egui::CollapsingHeader::new("Advanced address and port")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.draft.manual_address_override,
                    "Use the address I enter below in new invites",
                );
                ui.horizontal(|ui| {
                    ui.label("Reachable address");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.draft.advertised_host)
                            .hint_text("Detected automatically when possible")
                            .desired_width(300.0),
                    );
                    ui.label("Party port");
                    ui.add(egui::DragValue::new(&mut self.draft.peer_port).range(1024..=65535));
                });
                ui.label(RichText::new("Change these only for a manual router rule or a non-default network setup. New invites include both values.").small());
            });
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if *self.quit.lock().unwrap() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        let close = ctx.input(|i| i.viewport().close_requested());
        if close && !*self.quit.lock().unwrap() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        while let Ok(action) = self.tray_rx.try_recv() {
            match action {
                TrayAction::Open => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                #[cfg(target_os = "linux")]
                TrayAction::Quit => *self.quit.lock().unwrap() = true,
            }
        }
        egui::TopBottomPanel::top("spellbook-top")
            .frame(
                Frame::new()
                    .fill(PAPER_LIGHT)
                    .inner_margin(egui::Margin::symmetric(20, 15)),
            )
            .show(ctx, |ui| self.top(ui));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::symmetric(25, 18)),
            )
            .show(ctx, |ui| match self.tab {
                Tab::Overlay => self.overlay_ui(ui),
                Tab::Wizard => self.wizard_ui(ui),
                Tab::Party => self.party_ui(ui),
            });
        egui::TopBottomPanel::bottom("status")
            .frame(
                Frame::new()
                    .fill(PAPER_LIGHT)
                    .inner_margin(egui::Margin::symmetric(20, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✦").color(GOLD));
                    ui.label(RichText::new(&self.status).small().color(INK));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Quit WizRust101-OBS").clicked() {
                            *self.quit.lock().unwrap() = true;
                        }
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_millis(180));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        #[cfg(target_os = "linux")]
        if let TrayLifetime::Linux(tx) = &self._tray {
            let _ = tx.send(LinuxCommand::Stop);
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(16.0).strong().color(RED));
    ui.label(
        RichText::new(subtitle)
            .size(12.0)
            .color(Color32::from_rgb(103, 82, 62)),
    );
}
fn setting_toggle(ui: &mut egui::Ui, label: &str, value: &mut bool, help: &str) {
    Frame::new()
        .fill(PAPER_LIGHT)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(207, 180, 123)))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(value, "");
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).strong());
                    ui.label(
                        RichText::new(help)
                            .small()
                            .color(Color32::from_rgb(103, 82, 62)),
                    );
                });
            });
        });
}
fn brass_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(PAPER_LIGHT))
            .fill(RED)
            .stroke(Stroke::new(1.0_f32, GOLD))
            .corner_radius(egui::CornerRadius::same(8)),
    )
}
fn sigil(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
    let p = ui.painter();
    p.circle_filled(r.center(), 22.0, Color32::from_rgb(67, 48, 51));
    p.circle_stroke(r.center(), 20.0, Stroke::new(1.5_f32, GOLD));
    p.line_segment(
        [
            r.center() + egui::vec2(0.0, -13.0),
            r.center() + egui::vec2(0.0, 13.0),
        ],
        Stroke::new(2.0_f32, Color32::from_rgb(237, 204, 127)),
    );
    p.line_segment(
        [
            r.center() + egui::vec2(-11.0, 7.0),
            r.center() + egui::vec2(11.0, -7.0),
        ],
        Stroke::new(2.0_f32, Color32::from_rgb(237, 204, 127)),
    );
}
fn school_color(s: &str) -> Color32 {
    match s {
        "Fire" => Color32::from_rgb(159, 54, 47),
        "Ice" => Color32::from_rgb(65, 119, 163),
        "Storm" => Color32::from_rgb(111, 94, 164),
        "Myth" => Color32::from_rgb(198, 145, 56),
        "Life" => Color32::from_rgb(67, 126, 82),
        "Death" => Color32::from_rgb(75, 72, 92),
        _ => Color32::from_rgb(168, 122, 48),
    }
}

fn decode_icon(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut rgba)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err("application icons must be 8-bit RGBA PNGs".into());
    }
    rgba.truncate(info.buffer_size());
    Ok((rgba, info.width, info.height))
}

#[cfg(target_os = "linux")]
struct LinuxTray {
    tx: std::sync::mpsc::Sender<TrayAction>,
    display_name: String,
}
#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        let suffix = self
            .display_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>();
        format!("wizrust101-obs-{}-{}", suffix, std::process::id())
    }
    fn title(&self) -> String {
        format!("WizRust101-OBS — {}", self.display_name)
    }
    fn icon_name(&self) -> String {
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [32_u32, 64]
            .into_iter()
            .filter_map(|size| {
                let bytes: &[u8] = match size {
                    32 => include_bytes!("../assets/icons/sizes/32.png"),
                    _ => include_bytes!("../assets/icons/sizes/64.png"),
                };
                decode_icon(bytes).ok().map(|(mut data, width, height)| {
                    for pixel in data.chunks_exact_mut(4) {
                        pixel.rotate_right(1);
                    }
                    ksni::Icon {
                        width: width as i32,
                        height: height as i32,
                        data,
                    }
                })
            })
            .collect()
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayAction::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let tx = self.tx.clone();
        let open = StandardItem {
            label: "Open Spellbook Settings".into(),
            icon_name: "preferences-system".into(),
            activate: Box::new(move |_| {
                let _ = tx.send(TrayAction::Open);
            }),
            ..Default::default()
        };
        let tx = self.tx.clone();
        let quit = StandardItem {
            label: "Quit WizRust101-OBS".into(),
            icon_name: "application-exit".into(),
            activate: Box::new(move |_| {
                let _ = tx.send(TrayAction::Quit);
            }),
            ..Default::default()
        };
        vec![open.into(), quit.into()]
    }
}

#[cfg(target_os = "linux")]
fn install_tray(
    _ctx: Context,
    _quit: Arc<Mutex<bool>>,
    display_name: String,
) -> (Receiver<TrayAction>, TrayLifetime) {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread_tx = tx.clone();
    std::thread::spawn(move || {
        let service = LinuxTray {
            tx: thread_tx,
            display_name,
        };
        if let Ok(handle) = service.assume_sni_available(true).spawn() {
            while !matches!(stop_rx.recv(), Ok(LinuxCommand::Stop) | Err(_)) {}
            handle.shutdown().wait();
        }
    });
    (rx, TrayLifetime::Linux(stop_tx))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn install_tray(
    ctx: Context,
    quit: Arc<Mutex<bool>>,
    display_name: String,
) -> (Receiver<TrayAction>, TrayLifetime) {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };
    let (tx, rx) = mpsc::channel();
    let menu = Menu::new();
    let show = MenuItem::with_id(
        "show-settings",
        format!("Open {display_name} Settings"),
        true,
        None,
    );
    let exit = MenuItem::with_id(
        "quit-app",
        format!("Quit WizRust101-OBS ({display_name})"),
        true,
        None,
    );
    let _ = menu.append(&show);
    let _ = menu.append(&exit);
    let (pixels, width, height) = decode_icon(include_bytes!("../assets/icons/sizes/32.png"))
        .expect("bundled tray icon is a valid RGBA PNG");
    let icon = Icon::from_rgba(pixels, width, height).expect("bundled tray icon");
    let tray = TrayIconBuilder::new()
        .with_tooltip(format!("WizRust101-OBS — {display_name}"))
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .ok();
    let ctx_menu = ctx.clone();
    let quit_menu = quit.clone();
    let tx_menu = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        "show-settings" => {
            let _ = tx_menu.send(TrayAction::Open);
            ctx_menu.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx_menu.request_repaint();
        }
        "quit-app" => {
            *quit_menu.lock().unwrap() = true;
            ctx_menu.request_repaint();
        }
        _ => {}
    }));
    let ctx_click = ctx.clone();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            ctx_click.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx_click.send_viewport_cmd(ViewportCommand::Focus);
            ctx_click.request_repaint();
        }
    }));
    (
        rx,
        TrayLifetime::Native {
            _icon: tray.expect("Could not create the system tray icon"),
        },
    )
}

#[cfg(test)]
mod icon_tests {
    use super::decode_icon;

    #[test]
    fn bundled_app_icon_decodes_to_transparent_rgba() {
        let (rgba, width, height) =
            decode_icon(include_bytes!("../assets/icons/sizes/32.png")).unwrap();
        assert_eq!((width, height), (32, 32));
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        assert!(rgba.iter().skip(3).step_by(4).any(|alpha| *alpha == 0));
    }

    #[test]
    fn icon_decoder_rejects_malformed_png() {
        assert!(decode_icon(b"not a PNG").is_err());
    }
}
