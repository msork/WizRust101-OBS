use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use crate::{
    config::{AppConfig, CharacterProfile, SCHOOLS},
    peer::{self, PairingInvite},
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

pub fn run(shared: SharedState) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WizRust101-OBS • Spellbook")
            .with_inner_size([850.0, 690.0])
            .with_min_inner_size([700.0, 560.0])
            .with_visible(false),
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
    draft: AppConfig,
    tab: Tab,
    status: String,
    invite_text: String,
    import_text: String,
    import_label: String,
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
    fn new(shared: SharedState, ctx: Context) -> Self {
        let draft = shared.config.lock().unwrap().clone();
        let selected_profile = draft
            .active_profile
            .clone()
            .or_else(|| draft.profiles.first().map(|p| p.id.clone()))
            .unwrap_or_default();
        let quit = Arc::new(Mutex::new(false));
        let (tray_rx, tray) = install_tray(ctx, quit.clone());
        Self {
            shared,
            draft,
            tab: Tab::Overlay,
            status: "Ready • your settings stay on this device".into(),
            invite_text: String::new(),
            import_text: String::new(),
            import_label: String::new(),
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
        match self.draft.save() {
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
        let id = format!("wizard-{}", rand::random::<u32>());
        match peer::create_pairing(id) {
            Ok((credential, _)) => {
                let host = self.draft.advertised_host.trim();
                let url = if host.is_empty() {
                    format!(
                        "ws://YOUR_HOST_ADDRESS:{}/peer?peer_id={}",
                        peer::PEER_PORT,
                        credential.peer_id
                    )
                } else {
                    peer::invite_for(&credential, host)
                        .map(|i| i.url)
                        .unwrap_or_default()
                };
                let invite = PairingInvite {
                    peer_id: credential.peer_id.clone(),
                    secret: credential.secret.clone(),
                    url,
                };
                self.draft.peer_links.push(credential);
                self.invite_text = serde_json::to_string_pretty(&invite).unwrap_or_default();
                self.save();
                self.status =
                    "Pairing created. Share this secret only with the person you intend to connect."
                        .into()
            }
            Err(e) => self.status = e,
        }
    }
    fn import_invite(&mut self) {
        match peer::import_invite(&self.import_text, self.import_label.trim().to_owned()) {
            Ok(link) => {
                if let Some(existing) = self
                    .draft
                    .peer_links
                    .iter_mut()
                    .find(|p| p.peer_id == link.peer_id)
                {
                    *existing = link;
                } else {
                    self.draft.peer_links.push(link);
                }
                self.import_text.clear();
                self.import_label.clear();
                self.save();
                self.status =
                    "Paired connection saved. It will connect while the app is running.".into()
            }
            Err(e) => self.status = e,
        }
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
        ui.add_space(16.0);
        section(
            ui,
            "OBS BROWSER SOURCE",
            "Add this URL as a Browser Source • canvas 1920 × 1080",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("http://127.0.0.1:17841/overlay").monospace());
            if brass_button(ui, "Copy overlay URL").clicked() {
                ui.ctx().copy_text("http://127.0.0.1:17841/overlay".into());
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
            "PARTY PRESENCE",
            "Your wizard remains the prominent lead. Connected friends appear as smaller party cards.",
        );
        ui.add_space(8.0);
        setting_toggle(
            ui,
            "Enable my peer server",
            &mut self.draft.collaboration_server_enabled,
            "Off by default. Enables only the authenticated peer socket on port 17842.",
        );
        setting_toggle(
            ui,
            "Request UPnP port forwarding",
            &mut self.draft.upnp_port_forward,
            "Off by default. Only requested while your peer server is enabled.",
        );
        ui.add_enabled_ui(self.draft.collaboration_server_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Reachable host name or IP");
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.advertised_host)
                        .hint_text("Enter your reachable LAN or public address")
                        .desired_width(310.0),
                );
                if brass_button(ui, "Create invitation").clicked() {
                    self.create_invite();
                }
            });
            if !self.invite_text.is_empty() {
                ui.label("ONE-TIME DISPLAY • treat this pairing secret like a password");
                ui.add(
                    egui::TextEdit::multiline(&mut self.invite_text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                if ui.button("Copy invitation").clicked() {
                    ui.ctx().copy_text(self.invite_text.clone());
                    self.status = "Pairing invitation copied".into();
                }
            }
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new("PAIR WITH ANOTHER WIZRUST101-OBS INSTANCE")
                .strong()
                .color(RED),
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.import_label)
                    .hint_text("Friend label")
                    .desired_width(155.0),
            );
            if brass_button(ui, "Import invitation").clicked() {
                self.import_invite();
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.import_text)
                .hint_text("Paste the pairing JSON shared by the host")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if !self.draft.peer_links.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("PAIRED PEERS").strong().color(RED));
            let mut remove = None;
            for peer in &self.draft.peer_links {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{}  •  {}",
                        if peer.connect_url.is_some() {
                            "↔"
                        } else {
                            "Invite"
                        },
                        if peer.label.is_empty() {
                            &peer.peer_id
                        } else {
                            &peer.label
                        }
                    ));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(peer.peer_id.clone());
                    }
                });
            }
            if let Some(id) = remove {
                self.draft.peer_links.retain(|p| p.peer_id != id);
                self.shared.remove_peer(&id);
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Pairing uses Noise PSK encryption. Only an imported pairing secret is accepted; the peer socket cannot read OBS state or local configuration.").small().color(Color32::from_rgb(89,72,55)));
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

#[cfg(target_os = "linux")]
struct LinuxTray {
    tx: std::sync::mpsc::Sender<TrayAction>,
}
#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "wizrust101-obs".into()
    }
    fn title(&self) -> String {
        "WizRust101-OBS".into()
    }
    fn icon_name(&self) -> String {
        "applications-games".into()
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
fn install_tray(_ctx: Context, _quit: Arc<Mutex<bool>>) -> (Receiver<TrayAction>, TrayLifetime) {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread_tx = tx.clone();
    std::thread::spawn(move || {
        let service = LinuxTray { tx: thread_tx };
        if let Ok(handle) = service.assume_sni_available(true).spawn() {
            while !matches!(stop_rx.recv(), Ok(LinuxCommand::Stop) | Err(_)) {}
            handle.shutdown().wait();
        }
    });
    (rx, TrayLifetime::Linux(stop_tx))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn install_tray(ctx: Context, quit: Arc<Mutex<bool>>) -> (Receiver<TrayAction>, TrayLifetime) {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };
    let (tx, rx) = mpsc::channel();
    let menu = Menu::new();
    let show = MenuItem::with_id("show-settings", "Open Spellbook Settings", true, None);
    let exit = MenuItem::with_id("quit-app", "Quit WizRust101-OBS", true, None);
    let _ = menu.append(&show);
    let _ = menu.append(&exit);
    let mut pixels = vec![0_u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let inside = dx * dx + dy * dy < 210.0;
            let gold = dx.abs() < 2.0 || dy.abs() < 2.0 || (dx - dy).abs() < 2.0;
            let c = if !inside {
                [0, 0, 0, 0]
            } else if gold {
                [228, 187, 100, 255]
            } else {
                [68, 45, 50, 255]
            };
            let i = (y * 32 + x) * 4;
            pixels[i..i + 4].copy_from_slice(&c);
        }
    }
    let icon = Icon::from_rgba(pixels, 32, 32).expect("generated tray icon");
    let tray = TrayIconBuilder::new()
        .with_tooltip("WizRust101-OBS")
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
