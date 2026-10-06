#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

use crate::{
    config::{AppConfig, CharacterProfile, SCHOOLS, UiTheme},
    peer::{self},
    state::SharedState,
};
use eframe::egui::{
    self, Align, Color32, ComboBox, Context, Frame, Layout, RichText, Stroke, ViewportCommand,
};

const GOLD: Color32 = Color32::from_rgb(177, 132, 55);
const RED: Color32 = Color32::from_rgb(119, 48, 53);
const TRAY_OPEN_LABEL: &str = "Open Settings";
const TRAY_QUIT_LABEL: &str = "Quit WizRust101-OBS";

#[derive(Clone, Copy)]
struct Palette {
    ink: Color32,
    paper: Color32,
    surface: Color32,
    subtitle: Color32,
    edge: Color32,
    accent: Color32,
}

fn palette(theme: UiTheme) -> Palette {
    match theme {
        UiTheme::Light => Palette {
            ink: Color32::from_rgb(47, 34, 42),
            paper: Color32::from_rgb(239, 225, 190),
            surface: Color32::from_rgb(249, 240, 215),
            subtitle: Color32::from_rgb(103, 82, 62),
            edge: Color32::from_rgb(207, 180, 123),
            accent: RED,
        },
        UiTheme::Dark => Palette {
            ink: Color32::from_rgb(239, 226, 197),
            paper: Color32::from_rgb(39, 31, 37),
            surface: Color32::from_rgb(52, 41, 45),
            subtitle: Color32::from_rgb(197, 180, 151),
            edge: Color32::from_rgb(157, 119, 59),
            accent: Color32::from_rgb(232, 190, 111),
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overlay,
    Wizard,
    Party,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayAction {
    Open,
    Quit,
}

#[derive(Clone, Default)]
struct WindowControl {
    tx: Option<mpsc::Sender<NativeWindowCommand>>,
    #[cfg(target_os = "windows")]
    hidden: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "windows")]
    hwnd: Option<Arc<AtomicIsize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeWindowCommand {
    Hide,
    Show,
    Quit,
    #[cfg(target_os = "windows")]
    Stop,
}

#[cfg(target_os = "windows")]
struct NativeWindowWorker {
    stop: mpsc::Sender<NativeWindowCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WindowControl {
    fn hide(&self, ctx: &Context) {
        if let Some(tx) = &self.tx {
            #[cfg(target_os = "windows")]
            debug_viewport_state("in-app hide requested", ctx, self.native_hidden());
            let _ = tx.send(NativeWindowCommand::Hide);
            return;
        }
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Returns true when a Windows native controller handled the show request.
    fn show(&self, ctx: &Context) -> bool {
        if let Some(tx) = &self.tx {
            let _ = tx.send(NativeWindowCommand::Show);
            ctx.request_repaint();
            return true;
        }
        restore_settings_viewport(ctx);
        false
    }

    #[cfg(target_os = "windows")]
    fn native_hidden(&self) -> Option<bool> {
        self.hidden
            .as_ref()
            .map(|hidden| hidden.load(Ordering::SeqCst))
    }

    #[cfg(target_os = "windows")]
    fn set_native_handle(&self, hwnd: Option<isize>) {
        if let (Some(hwnd), Some(target)) = (hwnd, &self.hwnd) {
            target.store(hwnd, Ordering::SeqCst);
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn native_hidden(&self) -> Option<bool> {
        None
    }

    fn quit(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(NativeWindowCommand::Quit);
        }
    }
}

#[cfg(target_os = "windows")]
fn start_native_window_worker(
    hwnd: Option<isize>,
    repaint: Context,
    quit: Arc<Mutex<bool>>,
) -> (WindowControl, NativeWindowWorker) {
    let (tx, rx) = mpsc::channel();
    let handle = Arc::new(AtomicIsize::new(hwnd.unwrap_or_default()));
    let hidden = Arc::new(AtomicBool::new(false));
    let worker_handle = handle.clone();
    let worker_hidden = hidden.clone();
    let thread = std::thread::spawn(move || {
        while let Ok(command) = rx.recv() {
            if command == NativeWindowCommand::Stop {
                break;
            }
            if command == NativeWindowCommand::Quit {
                *quit.lock().unwrap() = true;
            }
            let hwnd = worker_handle.load(Ordering::SeqCst);
            if hwnd == 0 {
                debug_window_trace(&format!("{command:?} ignored: native HWND unavailable"));
                repaint.request_repaint();
                continue;
            }
            native_window_command(hwnd, command, &worker_hidden);
            if command != NativeWindowCommand::Hide {
                repaint.request_repaint();
                debug_window_trace("native show completed; egui repaint requested");
            }
        }
    });
    (
        WindowControl {
            tx: Some(tx.clone()),
            hidden: Some(hidden),
            hwnd: Some(handle),
        },
        NativeWindowWorker {
            stop: tx,
            thread: Some(thread),
        },
    )
}

#[cfg(target_os = "windows")]
fn native_window_command(hwnd: isize, command: NativeWindowCommand, hidden: &AtomicBool) {
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE,
            SetForegroundWindow, ShowWindow,
        },
    };

    let hwnd = hwnd as HWND;
    debug_window_trace(&format!(
        "native {command:?} before: visible={}, minimized={}, foreground={}",
        unsafe { IsWindowVisible(hwnd) != 0 },
        unsafe { IsIconic(hwnd) != 0 },
        unsafe { GetForegroundWindow() == hwnd },
    ));
    match command {
        NativeWindowCommand::Hide => {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            hidden.store(true, Ordering::SeqCst);
        }
        NativeWindowCommand::Show | NativeWindowCommand::Quit => {
            unsafe { ShowWindow(hwnd, SW_RESTORE) };
            unsafe {
                BringWindowToTop(hwnd);
                SetForegroundWindow(hwnd);
            }
            hidden.store(false, Ordering::SeqCst);
        }
        NativeWindowCommand::Stop => {}
    }
    debug_window_trace(&format!(
        "native {command:?} after: visible={}, minimized={}, foreground={}, hidden_flag={}",
        unsafe { IsWindowVisible(hwnd) != 0 },
        unsafe { IsIconic(hwnd) != 0 },
        unsafe { GetForegroundWindow() == hwnd },
        hidden.load(Ordering::SeqCst),
    ));
}

#[cfg(target_os = "windows")]
fn debug_window_trace(message: &str) {
    #[cfg(debug_assertions)]
    eprintln!("[settings-window] {message}");
    #[cfg(not(debug_assertions))]
    let _ = message;
}

#[cfg(target_os = "windows")]
fn debug_viewport_state(label: &str, ctx: &Context, native_hidden: Option<bool>) {
    #[cfg(debug_assertions)]
    {
        let (minimized, focused) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.minimized, viewport.focused)
        });
        eprintln!(
            "[settings-window] {label}: eframe-visible=not-exposed, minimized={minimized:?}, focused={focused:?}, native-hidden={native_hidden:?}"
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (label, ctx, native_hidden);
}

#[cfg(not(target_os = "windows"))]
fn debug_viewport_state(_label: &str, _ctx: &Context, _native_hidden: Option<bool>) {}

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
            .with_title(window_title(&display_name))
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
            install_theme(&cc.egui_ctx, shared.config.lock().unwrap().ui_theme);
            let native_hwnd = native_window_handle(cc);
            Ok(Box::new(SettingsApp::new(
                shared.clone(),
                cc.egui_ctx.clone(),
                config_path.clone(),
                overlay_url.clone(),
                display_name.clone(),
                demo_mode,
                native_hwnd,
            )))
        }),
    )?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn native_window_handle(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn frame_native_handle(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = frame.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn native_window_handle(_cc: &eframe::CreationContext<'_>) -> Option<isize> {
    None
}

fn install_theme(ctx: &Context, theme: UiTheme) {
    let colors = palette(theme);
    let mut v = match theme {
        UiTheme::Light => egui::Visuals::light(),
        UiTheme::Dark => egui::Visuals::dark(),
    };
    v.panel_fill = colors.paper;
    v.window_fill = colors.surface;
    v.extreme_bg_color = colors.surface;
    v.faint_bg_color = colors.paper;
    v.override_text_color = Some(colors.ink);
    v.selection.bg_fill = Color32::from_rgb(196, 158, 87);
    v.widgets.noninteractive.bg_fill = colors.paper;
    v.widgets.inactive.bg_fill = colors.surface;
    v.widgets.hovered.bg_fill = colors.edge;
    v.widgets.active.bg_fill = Color32::from_rgb(196, 158, 87);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, colors.edge);
    let control_ink = if theme == UiTheme::Dark {
        Color32::from_rgb(255, 231, 170)
    } else {
        Color32::from_rgb(61, 39, 37)
    };
    let control_edge = if theme == UiTheme::Dark {
        Color32::from_rgb(232, 190, 111)
    } else {
        Color32::from_rgb(142, 94, 42)
    };
    v.widgets.inactive.bg_stroke = Stroke::new(1.8_f32, control_edge);
    v.widgets.inactive.fg_stroke = Stroke::new(2.6_f32, control_ink);
    v.widgets.hovered.bg_fill = if theme == UiTheme::Dark {
        Color32::from_rgb(91, 67, 59)
    } else {
        Color32::from_rgb(231, 208, 158)
    };
    v.widgets.hovered.bg_stroke = Stroke::new(2.2_f32, control_edge);
    v.widgets.hovered.fg_stroke = Stroke::new(2.8_f32, control_ink);
    v.widgets.active.bg_fill = if theme == UiTheme::Dark {
        Color32::from_rgb(125, 78, 52)
    } else {
        Color32::from_rgb(218, 184, 112)
    };
    v.widgets.active.bg_stroke = Stroke::new(2.4_f32, control_edge);
    v.widgets.active.fg_stroke = Stroke::new(3.0_f32, control_ink);
    v.widgets.open = v.widgets.hovered;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 9.0);
        s.visuals.window_corner_radius = egui::CornerRadius::same(13);
    });
}

fn window_title(display_name: &str) -> String {
    if display_name == "WizRust101-OBS" {
        display_name.to_owned()
    } else {
        format!("WizRust101-OBS - {display_name}")
    }
}

struct SettingsApp {
    shared: SharedState,
    config_path: PathBuf,
    overlay_url: String,
    demo_mode: bool,
    demo_world: String,
    demo_zone: String,
    draft: AppConfig,
    app_icon: egui::TextureHandle,
    school_icons: HashMap<String, egui::TextureHandle>,
    tab: Tab,
    status: String,
    last_autosave: Instant,
    issued_invites: IssuedInvites,
    import_text: String,
    selected_profile: String,
    quit: Arc<Mutex<bool>>,
    tray_rx: Receiver<TrayAction>,
    window_control: WindowControl,
    startup_hide_pending: bool,
    last_native_hidden: Option<bool>,
    focus_after_restore: bool,
    _tray: TrayLifetime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IssuedInvite {
    peer_id: String,
    code: String,
    expires_at_unix: u64,
}

#[derive(Default)]
struct IssuedInvites(Vec<IssuedInvite>);

impl IssuedInvites {
    fn add(&mut self, invite: IssuedInvite) -> Result<(), &'static str> {
        if self.0.len() >= peer::MAX_GUESTS {
            return Err(
                "All three invites are in use. Remove a wizard or wait for an invite to expire.",
            );
        }
        self.0.push(invite);
        Ok(())
    }

    fn reconcile(
        &mut self,
        connected: &HashSet<String>,
        now: u64,
        capacity: usize,
    ) -> HashSet<String> {
        let mut invalidate = HashSet::new();
        self.0.retain(|invite| {
            if connected.contains(&invite.peer_id) {
                false
            } else if invite.expires_at_unix <= now {
                invalidate.insert(invite.peer_id.clone());
                false
            } else {
                true
            }
        });
        if self.0.len() > capacity {
            invalidate.extend(
                self.0
                    .split_off(capacity)
                    .into_iter()
                    .map(|invite| invite.peer_id),
            );
        }
        invalidate
    }
}

fn available_invite_slots(remote_members: usize) -> usize {
    peer::MAX_GUESTS.saturating_sub(remote_members.min(peer::MAX_GUESTS))
}

fn can_create_invite(unused_invites: usize, capacity: usize) -> bool {
    capacity > 0 && unused_invites < capacity
}

fn show_join_party_section(hosting: bool, joining: bool, remote_members: usize) -> bool {
    !hosting && (!joining || remote_members == 0)
}

fn school_icon_bytes(school: &str) -> &'static [u8] {
    match school.to_ascii_lowercase().as_str() {
        "fire" => include_bytes!("../assets/schools/fire.png"),
        "ice" => include_bytes!("../assets/schools/ice.png"),
        "storm" => include_bytes!("../assets/schools/storm.png"),
        "myth" => include_bytes!("../assets/schools/myth.png"),
        "life" => include_bytes!("../assets/schools/life.png"),
        "death" => include_bytes!("../assets/schools/death.png"),
        "balance" => include_bytes!("../assets/schools/balance.png"),
        _ => include_bytes!("../assets/schools/balance.png"),
    }
}

fn load_icon_texture(ctx: &Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let (rgba, width, height) = decode_icon(bytes).expect("bundled icon must be a valid PNG");
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    ctx.load_texture(name, image, Default::default())
}
enum TrayLifetime {
    #[cfg(target_os = "linux")]
    Linux {
        stop: std::sync::mpsc::Sender<LinuxCommand>,
        thread: Option<std::thread::JoinHandle<()>>,
    },
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    Native {
        _icon: tray_icon::TrayIcon,
        #[cfg(target_os = "windows")]
        worker: NativeWindowWorker,
    },
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
        native_hwnd: Option<isize>,
    ) -> Self {
        let draft = shared.config.lock().unwrap().clone();
        let app_icon = load_icon_texture(
            &ctx,
            "wzr-app-header",
            include_bytes!("../assets/icons/sizes/256.png"),
        );
        let school_icons = SCHOOLS
            .iter()
            .map(|school| {
                (
                    (*school).to_owned(),
                    load_icon_texture(
                        &ctx,
                        &format!("school-roster-{}", school.to_ascii_lowercase()),
                        school_icon_bytes(school),
                    ),
                )
            })
            .collect();
        let initial_state = shared.snapshot();
        let selected_profile = draft
            .active_profile
            .clone()
            .or_else(|| draft.profiles.first().map(|p| p.id.clone()))
            .unwrap_or_default();
        let quit = Arc::new(Mutex::new(false));
        let (tray_rx, tray, window_control) =
            install_tray(display_name.clone(), ctx, quit.clone(), native_hwnd);
        Self {
            shared,
            config_path,
            overlay_url,
            demo_mode,
            demo_world: initial_state.world.unwrap_or_else(|| "Wizard City".into()),
            demo_zone: initial_state.zone.unwrap_or_else(|| "The Commons".into()),
            draft,
            app_icon,
            school_icons,
            tab: Tab::Overlay,
            status: "Ready • your settings stay on this device".into(),
            last_autosave: Instant::now(),
            issued_invites: IssuedInvites::default(),
            import_text: String::new(),
            selected_profile,
            quit,
            tray_rx,
            window_control,
            startup_hide_pending: true,
            last_native_hidden: None,
            focus_after_restore: false,
            _tray: tray,
        }
    }
    fn save(&mut self) {
        self.persist(true);
    }
    fn save_profile_selection(&mut self) {
        if self.draft.validate().is_ok() {
            self.save();
            return;
        }

        // An incomplete new profile must not block activation of another
        // already-saved profile. Persist only the active ID in that case.
        let mut current = self.shared.config.lock().unwrap();
        if !current
            .profiles
            .iter()
            .any(|profile| profile.id == self.selected_profile)
        {
            return;
        }
        let mut updated = current.clone();
        updated.active_profile = Some(self.selected_profile.clone());
        match updated.save_to_path(&self.config_path) {
            Ok(()) => {
                *current = updated;
                drop(current);
                self.shared.publish_current();
            }
            Err(error) => self.status = format!("Could not save settings: {error}"),
        }
    }
    fn persist(&mut self, announce: bool) {
        let mut saved = self.shared.config.lock().unwrap();
        // The Party client can promote a link after an authenticated Welcome.
        // Keep that durable fact if the visible settings draft predates it.
        let established: std::collections::HashSet<String> = saved
            .peer_links
            .iter()
            .filter(|link| link.auto_reconnect)
            .map(|link| link.peer_id.clone())
            .collect();
        for link in &mut self.draft.peer_links {
            if established.contains(&link.peer_id) {
                link.auto_reconnect = true;
            }
        }
        if !self.draft.collaboration_server_enabled {
            self.draft.upnp_port_forward = false;
        }
        match self.draft.save_to_path(&self.config_path) {
            Ok(()) => {
                *saved = self.draft.clone();
                drop(saved);
                self.shared.publish_current();
                if announce {
                    self.status = "Settings saved on this device".into()
                }
            }
            Err(e) if announce => self.status = format!("Could not save settings: {e}"),
            Err(_) => {}
        }
    }
    fn profile_mut(&mut self) -> Option<&mut CharacterProfile> {
        self.draft
            .profiles
            .iter_mut()
            .find(|p| p.id == self.selected_profile)
    }
    fn create_invite(&mut self, ctx: &Context) {
        if !self.draft.collaboration_server_enabled {
            self.status = "Start hosting before creating an invite".into();
            return;
        }
        let members = self.shared.snapshot().party;
        self.reconcile_invites(&members);
        let capacity = available_invite_slots(members.len());
        if capacity == 0 {
            self.status = "The Party is full. No more invites can be created.".into();
            return;
        }
        if self.issued_invites.0.len() >= capacity {
            self.status = format!("All {capacity} available guest slots already have invites.");
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
                        let code = match peer::encode_invite(&invite) {
                            Ok(code) => code,
                            Err(error) => {
                                self.status = error;
                                return;
                            }
                        };
                        if let Err(error) = self.issued_invites.add(IssuedInvite {
                            peer_id: credential.peer_id.clone(),
                            code: code.clone(),
                            expires_at_unix: invite.expires_at_unix,
                        }) {
                            self.status = error.into();
                            return;
                        }
                        self.draft.peer_links.push(credential);
                        self.save();
                        ctx.copy_text(code);
                        self.status =
                            "New party invite copied. You can copy it again below.".into();
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
                let peer_id = link.peer_id.clone();
                self.draft.collaboration_server_enabled = false;
                self.draft.upnp_port_forward = false;
                self.draft.peer_links.clear();
                self.draft.peer_links.push(link);
                self.shared.clear_party();
                self.shared.set_party_status(None);
                self.save();
                self.shared.request_party_join(peer_id);
                self.status = "Joining party. Waiting for the host…".into()
            }
            Err(e) => self.status = e,
        }
    }
    fn host_party(&mut self, ctx: &Context) {
        if self.draft.active_profile.is_none() {
            self.status = "Choose a primary wizard before hosting a party".into();
            return;
        }
        self.draft.peer_links.clear();
        self.issued_invites.0.clear();
        self.shared.clear_party();
        self.shared.set_party_status(None);
        self.draft.collaboration_server_enabled = true;
        self.create_invite(ctx);
    }

    fn reconcile_invites(&mut self, members: &[crate::state::WizardPresence]) {
        let connected: HashSet<_> = members
            .iter()
            .map(|member| member.peer_id.clone())
            .collect();
        let capacity = available_invite_slots(members.len());
        let invalidated = self
            .issued_invites
            .reconcile(&connected, peer::unix_now(), capacity);
        if !invalidated.is_empty() {
            self.draft
                .peer_links
                .retain(|link| !invalidated.contains(&link.peer_id));
            // The peer acceptor reads this shared config directly, so publish
            // revocation now rather than waiting for the periodic autosave.
            self.persist(false);
        }
    }
    fn leave_party(&mut self) {
        self.draft.collaboration_server_enabled = false;
        self.draft.upnp_port_forward = false;
        self.draft.peer_links.clear();
        self.issued_invites.0.clear();
        self.shared.clear_party();
        self.shared.set_party_status(None);
        self.save();
        self.status = "You left the party".into();
    }
    fn top(&mut self, ui: &mut egui::Ui) {
        let colors = palette(self.draft.ui_theme);
        let app_icon = self.app_icon.id();
        ui.horizontal(|ui| {
            ui.image((app_icon, egui::vec2(48.0, 48.0)));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("WIZRUST101 • OBS SPELLBOOK")
                        .size(22.0)
                        .strong()
                        .color(colors.accent),
                );
                ui.label(
                    RichText::new("Character and location for your Wizard101 stream")
                        .size(13.0)
                        .color(colors.subtitle),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Minimize to tray").clicked() {
                    self.window_control.hide(ui.ctx());
                }
                ui.selectable_value(&mut self.draft.ui_theme, UiTheme::Dark, "Dark");
                ui.selectable_value(&mut self.draft.ui_theme, UiTheme::Light, "Light");
                ui.label(RichText::new("Appearance").small().color(colors.subtitle));
            });
        });
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Overlay, "Overlay"),
                (Tab::Wizard, "My Wizard"),
                (Tab::Party, "Party"),
            ] {
                let selected = self.tab == tab;
                let response = ui.selectable_label(
                    selected,
                    RichText::new(label).strong().color(if selected {
                        colors.accent
                    } else {
                        colors.ink
                    }),
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
            "Keep the game visible with a compact overlay near the edge.",
        );
        ui.add_space(8.0);
        let o = &mut self.draft.overlay;
        setting_toggle(
            ui,
            "Character and location plaque",
            &mut o.character_location,
            "Show your chosen wizard with the live world and location.",
        );
        setting_toggle(
            ui,
            "Zone transition",
            &mut o.zone_transition,
            "Briefly reveal a new location when the game changes zones.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("X position");
            ui.add(egui::Slider::new(&mut o.x_percent, 0.0..=100.0).suffix("%"));
            ui.label("Y position");
            ui.add(egui::Slider::new(&mut o.y_percent, 0.0..=100.0).suffix("%"));
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            ui.add(egui::Slider::new(&mut o.scale, 0.25..=3.0));
            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut o.opacity, 0.0..=1.0));
        });
        ui.horizontal(|ui| {
            ui.label("Transition duration");
            ui.add(egui::Slider::new(&mut o.transition_seconds, 1.0..=20.0).suffix(" sec"));
        });
        if self.demo_mode {
            ui.add_space(12.0);
            section(
                ui,
                "MOCK LOCATION",
                "Preview world and location changes without opening Wizard101.",
            );
            ui.horizontal(|ui| {
                ui.label("World");
                ui.add(egui::TextEdit::singleline(&mut self.demo_world).desired_width(160.0));
                ui.label("Location");
                ui.add(egui::TextEdit::singleline(&mut self.demo_zone).desired_width(190.0));
                if brass_button(ui, "Apply location").clicked() {
                    let raw = format!("mock/{}/{}", self.demo_world, self.demo_zone);
                    self.shared
                        .set_demo_state(&self.demo_world, &self.demo_zone, &raw);
                    self.status = "Mock location updated".into();
                }
                if ui.button("End mock session").clicked() {
                    self.shared.stop();
                    self.status = "Mock session ended".into();
                }
            });
        }
        ui.add_space(16.0);
        section(
            ui,
            "OBS BROWSER SOURCE",
            "Add this URL as a Browser Source at your canvas resolution.",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.overlay_url).monospace());
            if brass_button(ui, "Copy overlay URL").clicked() {
                ui.ctx().copy_text(self.overlay_url.clone());
                self.status = "Overlay URL copied".into();
            }
        });
    }
    fn wizard_ui(&mut self, ui: &mut egui::Ui) {
        section(
            ui,
            "YOUR CHARACTER",
            "Enter the character name exactly as it appears in Wizard101. This name and school are configured here, not detected from the game.",
        );
        let previous_selection = self.selected_profile.clone();
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
                    for profile in &self.draft.profiles {
                        ui.selectable_value(
                            &mut self.selected_profile,
                            profile.id.clone(),
                            &profile.name,
                        );
                    }
                });
            if brass_button(ui, "Add character").clicked() {
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
        if self.selected_profile != previous_selection
            && activate_selected_profile(&mut self.draft, &self.selected_profile)
        {
            self.save_profile_selection();
        }
        let mut save_profile = false;
        let mut remove_profile = false;
        if let Some(profile) = self.profile_mut() {
            ui.add_space(12.0);
            ui.label("CHARACTER NAME");
            ui.add(
                egui::TextEdit::singleline(&mut profile.name)
                    .hint_text("Name shown in Wizard101")
                    .desired_width(460.0),
            );
            ui.add_space(8.0);
            ui.label("SCHOOL");
            ComboBox::from_id_salt("school")
                .selected_text(&profile.school)
                .show_ui(ui, |ui| {
                    for school in SCHOOLS {
                        ui.selectable_value(&mut profile.school, school.to_string(), school);
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Chosen school");
                let color = crate::school_palette::primary_color(&profile.school);
                ui.painter().circle_filled(
                    ui.cursor().left_top() + egui::vec2(12.0, 12.0),
                    10.0,
                    color,
                );
                let secondary = crate::school_palette::colors(&profile.school)
                    .map(|colors| parse_school_color(&colors.secondary))
                    .unwrap_or(color);
                ui.painter().circle_stroke(
                    ui.cursor().left_top() + egui::vec2(12.0, 12.0),
                    10.0,
                    Stroke::new(2.0_f32, secondary),
                );
                ui.add_space(28.0);
                ui.label(RichText::new(&profile.school).strong());
            });
            save_profile = brass_button(ui, "Save Profile").clicked();
            remove_profile = ui.button("Remove Profile").clicked();
        } else {
            ui.add_space(16.0);
            ui.label("Add a character profile to set the name and school for your overlay.");
        }
        if save_profile {
            self.save();
        }
        if remove_profile {
            self.draft
                .profiles
                .retain(|profile| profile.id != self.selected_profile);
            self.selected_profile = self
                .draft
                .profiles
                .first()
                .map(|profile| profile.id.clone())
                .unwrap_or_default();
            self.draft.active_profile = self
                .draft
                .profiles
                .first()
                .map(|profile| profile.id.clone());
            self.save();
        }
    }
    fn party_ui(&mut self, ui: &mut egui::Ui) {
        let colors = palette(self.draft.ui_theme);
        section(
            ui,
            "YOUR WIZARDS, TOGETHER",
            "Share a live roster while each stream keeps its own wizard in front.",
        );
        ui.add_space(10.0);
        let hosting = self.draft.collaboration_server_enabled;
        let joining = self
            .draft
            .peer_links
            .iter()
            .any(|p| p.connect_url.is_some());
        let members = self.shared.snapshot().party;
        self.reconcile_invites(&members);
        let occupancy = party_occupancy(members.len());
        let invite_capacity = available_invite_slots(members.len());
        Frame::new()
            .fill(colors.surface)
            .stroke(Stroke::new(1.0_f32, colors.edge))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let invite_expired = self.draft.peer_links.iter().any(|peer| {
                            peer.connect_url.is_some()
                                && peer
                                    .expires_at_unix
                                    .is_some_and(|expires| expires <= peer::unix_now())
                        });
                        let (title, detail) = if hosting {
                            ("Hosting Party", "Create up to three single-use invites")
                        } else if joining && members.is_empty() && invite_expired {
                            ("Invite Expired", "Paste a fresh party invite to reconnect")
                        } else if joining && members.is_empty() {
                            ("Joining Party", "Waiting for the host to return")
                        } else if joining {
                            ("In a Party", "Your wizard is connected")
                        } else {
                            ("No Party Yet", "Host a party or paste an invite to join")
                        };
                        ui.label(RichText::new(title).strong().color(colors.accent));
                        ui.label(RichText::new(detail).small().color(colors.subtitle));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if hosting {
                            let can_create =
                                can_create_invite(self.issued_invites.0.len(), invite_capacity);
                            ui.add_enabled_ui(can_create, |ui| {
                                if brass_button(ui, "Create Invite").clicked() {
                                    self.create_invite(ui.ctx());
                                }
                            });
                        } else if !joining && brass_button(ui, "Host Party").clicked() {
                            self.host_party(ui.ctx());
                        }
                        if (hosting || joining) && ui.button("Leave Party").clicked() {
                            self.leave_party();
                        }
                    });
                });
            });

        if hosting {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "Unused invites {}/{}",
                    self.issued_invites.0.len(),
                    invite_capacity
                ))
                .strong()
                .color(colors.accent),
            );
            if self.issued_invites.0.is_empty() {
                ui.label(
                    RichText::new(if invite_capacity == 0 {
                        "No guest slots are available while the Party is full."
                    } else {
                        "No unused invites. Create one to copy it to the clipboard."
                    })
                    .small()
                    .color(colors.subtitle),
                );
            }
            for invite in self.issued_invites.0.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Unused invite · {}", invite.peer_id)).small());
                    if brass_button(ui, "Copy").clicked() {
                        ui.ctx().copy_text(invite.code.clone());
                        self.status = "Party invite copied".into();
                    }
                });
            }
        }

        if show_join_party_section(hosting, joining, members.len()) {
            ui.add_space(12.0);
            ui.label(RichText::new("JOIN A PARTY").strong().color(colors.accent));
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
        }

        if let Some(message) = self.shared.party_status() {
            ui.label(RichText::new(message).strong().color(colors.accent));
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("PARTY ROSTER").strong().color(colors.accent));
            ui.label(
                RichText::new(format!("Party {occupancy}/{}", peer::MAX_PARTY_SIZE))
                    .small()
                    .color(colors.subtitle),
            );
        });
        if members.is_empty() {
            ui.label(
                RichText::new("Connected wizards will appear here.")
                    .small()
                    .color(colors.subtitle),
            );
        } else {
            for member in &members {
                ui.horizontal(|ui| {
                    if let Some(icon) = self.school_icons.get(&member.school) {
                        ui.image((icon.id(), egui::vec2(22.0, 22.0)));
                    }
                    let indicator_color = if member.active {
                        Color32::from_rgb(93, 153, 73)
                    } else {
                        colors.subtitle
                    };
                    let (indicator, _) =
                        ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(indicator.center(), 4.0, indicator_color);
                    ui.label(
                        RichText::new(&member.name)
                            .strong()
                            .color(palette(self.draft.ui_theme).ink),
                    );
                    ui.label(RichText::new(&member.school).small());
                    let status = if member.active {
                        format!(
                            "In game: {}",
                            [member.world.as_deref(), member.zone.as_deref()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" - ")
                        )
                    } else {
                        "Connected".into()
                    };
                    ui.label(RichText::new(status).small().color(colors.subtitle));
                });
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
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::Checkbox::new(
                        &mut self.draft.manual_address_override,
                        "Use the address I enter below in new invites",
                    ),
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
    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        self.window_control
            .set_native_handle(frame_native_handle(frame));
        install_theme(ctx, self.draft.ui_theme);
        if self.startup_hide_pending {
            self.startup_hide_pending = false;
            self.window_control.hide(ctx);
        }
        if self.focus_after_restore {
            // Focus has no effect on hidden or minimized native viewports.
            // This frame runs after the preceding restore commands were applied.
            focus_settings_viewport(ctx);
            self.focus_after_restore = false;
        }
        if *self.quit.lock().unwrap() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        let close = ctx.input(|i| i.viewport().close_requested());
        if close && !*self.quit.lock().unwrap() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.window_control.hide(ctx);
            debug_viewport_state(
                "close-to-tray intercepted",
                ctx,
                self.window_control.native_hidden(),
            );
        }
        while let Ok(action) = self.tray_rx.try_recv() {
            match action {
                TrayAction::Open => {
                    debug_viewport_state(
                        "tray Open consumed by UI",
                        ctx,
                        self.window_control.native_hidden(),
                    );
                    #[cfg(not(target_os = "windows"))]
                    {
                        self.focus_after_restore = !self.window_control.show(ctx);
                        ctx.request_repaint();
                    }
                }
                TrayAction::Quit => {
                    #[cfg(target_os = "windows")]
                    debug_window_trace("tray Quit consumed by UI");
                    signal_quit(&self.quit, ctx);
                }
            }
        }
        egui::TopBottomPanel::top("spellbook-top")
            .frame(
                Frame::new()
                    .fill(palette(self.draft.ui_theme).surface)
                    .inner_margin(egui::Margin::symmetric(20, 15)),
            )
            .show(ctx, |ui| self.top(ui));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette(self.draft.ui_theme).paper)
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
                    .fill(palette(self.draft.ui_theme).surface)
                    .inner_margin(egui::Margin::symmetric(20, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&self.status)
                            .small()
                            .color(palette(self.draft.ui_theme).subtitle),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(TRAY_QUIT_LABEL).clicked() {
                            signal_quit(&self.quit, ctx);
                        }
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_millis(180));
        let hidden = self.window_control.native_hidden();
        if hidden != self.last_native_hidden {
            debug_viewport_state("frame observed native visibility transition", ctx, hidden);
            self.last_native_hidden = hidden;
        }
        let saved = self.shared.config.lock().unwrap().clone();
        if self.draft != saved
            && self.draft.validate().is_ok()
            && self.last_autosave.elapsed() >= Duration::from_millis(500)
        {
            self.persist(false);
            self.last_autosave = Instant::now();
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        #[cfg(target_os = "linux")]
        if let TrayLifetime::Linux { stop, .. } = &self._tray {
            let _ = stop.send(LinuxCommand::Stop);
        }
    }
}

impl Drop for TrayLifetime {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let TrayLifetime::Linux { stop, thread } = self {
            let _ = stop.send(LinuxCommand::Stop);
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
        #[cfg(target_os = "windows")]
        match self {
            TrayLifetime::Native { worker, .. } => {
                let _ = worker.stop.send(NativeWindowCommand::Stop);
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
            }
        }
    }
}

fn signal_quit(quit: &Arc<Mutex<bool>>, ctx: &Context) {
    *quit.lock().unwrap() = true;
    ctx.send_viewport_cmd(ViewportCommand::Close);
    ctx.request_repaint();
}

fn restore_settings_viewport(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
}

fn focus_settings_viewport(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Focus);
}

fn activate_selected_profile(config: &mut AppConfig, profile_id: &str) -> bool {
    if config
        .profiles
        .iter()
        .any(|profile| profile.id == profile_id)
    {
        config.active_profile = Some(profile_id.to_owned());
        true
    } else {
        false
    }
}

fn section(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    let title_color = if ui.visuals().dark_mode {
        Color32::from_rgb(232, 190, 111)
    } else {
        RED
    };
    let subtitle_color = ui.visuals().text_color().gamma_multiply(0.78);
    ui.label(RichText::new(title).size(16.0).strong().color(title_color));
    ui.label(RichText::new(subtitle).size(12.0).color(subtitle_color));
}
fn setting_toggle(ui: &mut egui::Ui, label: &str, value: &mut bool, help: &str) {
    let visuals = ui.visuals().clone();
    Frame::new()
        .fill(visuals.window_fill)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_sized([30.0, 30.0], egui::Checkbox::new(value, ""));
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).strong());
                    ui.label(
                        RichText::new(help)
                            .small()
                            .color(visuals.text_color().gamma_multiply(0.78)),
                    );
                });
            });
        });
}
fn brass_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let text_color = if ui.visuals().dark_mode {
        Color32::from_rgb(250, 236, 204)
    } else {
        Color32::from_rgb(249, 240, 215)
    };
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(text_color))
            .fill(RED)
            .stroke(Stroke::new(1.0_f32, GOLD))
            .corner_radius(egui::CornerRadius::same(8)),
    )
}
fn parse_school_color(value: &str) -> Color32 {
    let rgb = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(0x4f4951);
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn party_occupancy(remote_members: usize) -> usize {
    (remote_members + 1).min(peer::MAX_PARTY_SIZE)
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
    repaint: Context,
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
        "WizRust101-OBS".into()
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
        dispatch_tray_action(&self.tx, &self.repaint, TrayAction::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let open = StandardItem {
            label: TRAY_OPEN_LABEL.into(),
            icon_name: "preferences-system".into(),
            activate: Box::new(move |_| {
                dispatch_tray_action(&tx, &repaint, TrayAction::Open);
            }),
            ..Default::default()
        };
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let quit = StandardItem {
            label: TRAY_QUIT_LABEL.into(),
            icon_name: "application-exit".into(),
            activate: Box::new(move |_| {
                dispatch_tray_action(&tx, &repaint, TrayAction::Quit);
            }),
            ..Default::default()
        };
        vec![open.into(), quit.into()]
    }
}

#[cfg(target_os = "linux")]
fn install_tray(
    display_name: String,
    repaint: Context,
    _quit: Arc<Mutex<bool>>,
    _native_hwnd: Option<isize>,
) -> (Receiver<TrayAction>, TrayLifetime, WindowControl) {
    use ksni::blocking::TrayMethods;
    let (tx, rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread_tx = tx.clone();
    let tray_thread = std::thread::spawn(move || {
        let service = LinuxTray {
            tx: thread_tx,
            repaint,
            display_name,
        };
        if let Ok(handle) = service.assume_sni_available(true).spawn() {
            while !matches!(stop_rx.recv(), Ok(LinuxCommand::Stop) | Err(_)) {}
            handle.shutdown().wait();
        }
    });
    (
        rx,
        TrayLifetime::Linux {
            stop: stop_tx,
            thread: Some(tray_thread),
        },
        WindowControl::default(),
    )
}

fn dispatch_tray_action(tx: &mpsc::Sender<TrayAction>, repaint: &Context, action: TrayAction) {
    let _ = tx.send(action);
    repaint.request_repaint();
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn install_tray(
    _display_name: String,
    repaint: Context,
    quit: Arc<Mutex<bool>>,
    native_hwnd: Option<isize>,
) -> (Receiver<TrayAction>, TrayLifetime, WindowControl) {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };
    let (tx, rx) = mpsc::channel();
    #[cfg(target_os = "windows")]
    let (window_control, worker) = start_native_window_worker(native_hwnd, repaint.clone(), quit);
    #[cfg(target_os = "macos")]
    let window_control = WindowControl::default();
    let menu = Menu::new();
    let show = MenuItem::with_id("show-settings", TRAY_OPEN_LABEL, true, None);
    let exit = MenuItem::with_id("quit-app", TRAY_QUIT_LABEL, true, None);
    let _ = menu.append(&show);
    let _ = menu.append(&exit);
    let (pixels, width, height) = decode_icon(include_bytes!("../assets/icons/sizes/32.png"))
        .expect("bundled tray icon is a valid RGBA PNG");
    let icon = Icon::from_rgba(pixels, width, height).expect("bundled tray icon");
    let tray = TrayIconBuilder::new()
        .with_tooltip("WizRust101-OBS")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .ok();
    let tx_menu = tx.clone();
    let repaint_menu = repaint.clone();
    let menu_control = window_control.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        "show-settings" => {
            menu_control.show(&repaint_menu);
            dispatch_tray_action(&tx_menu, &repaint_menu, TrayAction::Open);
        }
        "quit-app" => {
            menu_control.quit();
            dispatch_tray_action(&tx_menu, &repaint_menu, TrayAction::Quit);
        }
        _ => {}
    }));
    let tx_click = tx.clone();
    let repaint_click = repaint.clone();
    let click_control = window_control.clone();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            let _ = click_control.show(&repaint_click);
            dispatch_tray_action(&tx_click, &repaint_click, TrayAction::Open);
        }
    }));
    (
        rx,
        TrayLifetime::Native {
            _icon: tray.expect("Could not create the system tray icon"),
            #[cfg(target_os = "windows")]
            worker,
        },
        window_control,
    )
}

#[cfg(test)]
mod icon_tests {
    use super::{
        HashSet, activate_selected_profile, decode_icon, focus_settings_viewport, install_theme,
        restore_settings_viewport, signal_quit, window_title,
    };
    use crate::config::{AppConfig, CharacterProfile, UiTheme};
    use eframe::egui::{Context, RawInput, ViewportCommand, ViewportId};
    use std::sync::{Arc, Mutex};

    #[test]
    fn tray_labels_are_short_and_hide_http_details() {
        assert_eq!(super::TRAY_OPEN_LABEL, "Open Settings");
        assert_eq!(super::TRAY_QUIT_LABEL, "Quit WizRust101-OBS");
        assert_eq!(window_title("WizRust101-OBS"), "WizRust101-OBS");
        assert!(!window_title("WizRust101-OBS").contains("HTTP"));
    }

    #[test]
    fn quit_action_requests_app_close() {
        let quit = Arc::new(Mutex::new(false));
        signal_quit(&quit, &Context::default());
        assert!(*quit.lock().unwrap());
    }

    #[test]
    fn restore_unminimizes_and_shows_before_focus_is_requested() {
        let ctx = Context::default();
        let restored = ctx.run(RawInput::default(), restore_settings_viewport);
        let commands = &restored.viewport_output[&ViewportId::ROOT].commands;
        assert!(matches!(
            commands.as_slice(),
            [
                ViewportCommand::Minimized(false),
                ViewportCommand::Visible(true)
            ]
        ));

        let focused = ctx.run(RawInput::default(), focus_settings_viewport);
        let commands = &focused.viewport_output[&ViewportId::ROOT].commands;
        assert!(matches!(commands.as_slice(), [ViewportCommand::Focus]));
    }

    #[test]
    fn checkbox_checked_hover_and_focus_colors_are_visible_in_both_themes() {
        for theme in [UiTheme::Light, UiTheme::Dark] {
            let ctx = Context::default();
            install_theme(&ctx, theme);
            let visuals = ctx.style().visuals.clone();
            let widgets = &visuals.widgets;
            assert!(widgets.inactive.bg_stroke.width >= 1.8);
            assert!(widgets.hovered.bg_stroke.width >= 2.2);
            assert!(widgets.active.bg_stroke.width >= 2.4);
            assert_eq!(widgets.active.fg_stroke.width, 3.0);
            assert_ne!(widgets.inactive.fg_stroke.color, widgets.inactive.bg_fill);
            assert_ne!(widgets.hovered.bg_stroke.color, widgets.hovered.bg_fill);
            assert_ne!(widgets.active.fg_stroke.color, widgets.active.bg_fill);
        }
    }

    #[test]
    fn selecting_a_saved_profile_immediately_changes_active_profile() {
        let config_profiles = ["first", "second"]
            .into_iter()
            .map(|id| CharacterProfile {
                id: id.into(),
                name: format!("Wizard {id}"),
                school: "Life".into(),
                ..Default::default()
            })
            .collect();
        let mut config = AppConfig {
            profiles: config_profiles,
            active_profile: Some("first".into()),
            ..Default::default()
        };

        assert!(activate_selected_profile(&mut config, "second"));
        assert_eq!(config.active_profile.as_deref(), Some("second"));
        assert!(!activate_selected_profile(&mut config, "missing"));
        assert_eq!(config.active_profile.as_deref(), Some("second"));
    }

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

    #[test]
    fn tray_queue_keeps_repeated_open_and_quit_actions_independent() {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..500 {
            super::dispatch_tray_action(&tx, &Context::default(), super::TrayAction::Open);
        }
        super::dispatch_tray_action(&tx, &Context::default(), super::TrayAction::Quit);
        for _ in 0..500 {
            assert_eq!(rx.try_recv(), Ok(super::TrayAction::Open));
        }
        assert_eq!(rx.try_recv(), Ok(super::TrayAction::Quit));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn twenty_native_hide_open_cycles_leave_eframe_visible_and_quit_queued() {
        let (tx, rx) = std::sync::mpsc::channel();
        let control = super::WindowControl {
            tx: Some(tx),
            ..Default::default()
        };
        let ctx = Context::default();
        for _ in 0..20 {
            let hidden = ctx.run(RawInput::default(), |ctx| control.hide(ctx));
            assert!(
                hidden.viewport_output[&ViewportId::ROOT]
                    .commands
                    .is_empty()
            );
            assert_eq!(rx.try_recv(), Ok(super::NativeWindowCommand::Hide));

            let shown = ctx.run(RawInput::default(), |ctx| {
                assert!(control.show(ctx));
            });
            assert!(shown.viewport_output[&ViewportId::ROOT].commands.is_empty());
            assert_eq!(rx.try_recv(), Ok(super::NativeWindowCommand::Show));
        }
        control.quit();
        assert_eq!(rx.try_recv(), Ok(super::NativeWindowCommand::Quit));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn party_occupancy_counts_local_wizard_and_caps_at_four() {
        assert_eq!(super::party_occupancy(0), 1);
        assert_eq!(super::party_occupancy(2), 3);
        assert_eq!(super::party_occupancy(3), 4);
    }

    #[test]
    fn party_invites_are_capped_and_used_codes_stay_removed_after_leave() {
        let mut invites = super::IssuedInvites::default();
        for index in 0..crate::peer::MAX_GUESTS {
            invites
                .add(super::IssuedInvite {
                    peer_id: format!("guest-{index}"),
                    code: format!("code-{index}"),
                    expires_at_unix: 200,
                })
                .unwrap();
        }
        assert_eq!(invites.0.len(), 3);
        assert!(
            invites
                .add(super::IssuedInvite {
                    peer_id: "fourth".into(),
                    code: "code-fourth".into(),
                    expires_at_unix: 200,
                })
                .is_err()
        );

        let revoked = invites.reconcile(&["guest-1".to_owned()].into(), 100, 3);
        assert!(
            revoked.is_empty(),
            "a used invitation stays authorized for reconnect"
        );
        assert_eq!(invites.0.len(), 2);
        invites.reconcile(&HashSet::new(), 100, 3);
        assert_eq!(
            invites.0.len(),
            2,
            "a used invite must not reappear after leave"
        );
        let expired = invites.reconcile(&HashSet::new(), 200, 3);
        assert!(invites.0.is_empty(), "expired invites are no longer active");
        assert_eq!(expired.len(), 2);
    }

    #[test]
    fn unused_invite_capacity_tracks_party_occupancy_from_one_to_four() {
        for remote_members in 0..crate::peer::MAX_GUESTS {
            assert_eq!(
                super::available_invite_slots(remote_members),
                crate::peer::MAX_GUESTS - remote_members,
                "Party {}/4 should have the matching number of invite slots",
                remote_members + 1
            );
        }
        assert_eq!(super::available_invite_slots(crate::peer::MAX_GUESTS), 0);
        assert!(super::can_create_invite(0, 3));
        assert!(!super::can_create_invite(3, 3));
        assert!(
            !super::can_create_invite(0, 0),
            "the full-party button is disabled"
        );
    }

    #[test]
    fn party_host_join_sections_are_mutually_exclusive() {
        assert!(!super::show_join_party_section(true, false, 0));
        assert!(!super::show_join_party_section(false, true, 1));
        assert!(super::show_join_party_section(false, false, 0));
        assert!(
            super::show_join_party_section(false, true, 0),
            "a rejected or pending join remains available for explicit retry"
        );
    }

    #[test]
    fn joining_member_revokes_unused_invites_that_exceed_remaining_slots() {
        let mut invites = super::IssuedInvites::default();
        for index in 0..3 {
            invites
                .add(super::IssuedInvite {
                    peer_id: format!("unused-{index}"),
                    code: format!("invite-{index}"),
                    expires_at_unix: 300,
                })
                .unwrap();
        }
        // A previously invited wizard reconnects, reducing spare capacity to one.
        let connected = ["reconnected-member".to_owned()].into();
        let revoked = invites.reconcile(&connected, 100, 1);
        assert_eq!(invites.0.len(), 1);
        assert_eq!(revoked.len(), 2);
        assert!(!revoked.contains("unused-0"));
        assert!(revoked.contains("unused-1"));
        assert!(revoked.contains("unused-2"));

        let full = ["reconnected-member".to_owned(), "another-member".to_owned()]
            .into_iter()
            .collect();
        let revoked = invites.reconcile(&full, 100, 0);
        assert_eq!(invites.0.len(), 0);
        assert_eq!(revoked, ["unused-0".to_owned()].into());
    }

    #[test]
    fn party_roster_rows_use_school_icon_and_status_dot_instead_of_glyph_marks() {
        let source = include_str!("ui.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let roster = production.split("PARTY ROSTER").nth(1).unwrap();
        assert!(roster.contains("school_icons.get(&member.school)"));
        assert!(roster.contains("circle_filled(indicator.center(), 4.0"));
        assert!(!roster.contains("RichText::new(\"•\")"));
        assert!(!roster.contains("RichText::new(\"●\")"));
    }

    #[test]
    fn app_header_uses_the_executable_icon_asset() {
        let source = include_str!("ui.rs");
        assert!(source.contains("include_bytes!(\"../assets/icons/sizes/256.png\")"));
        assert!(source.contains("ui.image((app_icon, egui::vec2(48.0, 48.0)))"));
    }

    #[test]
    fn roster_school_icons_use_the_bundled_school_assets() {
        for school in crate::config::SCHOOLS {
            let (rgba, width, height) =
                super::decode_icon(super::school_icon_bytes(school)).unwrap();
            assert!(width > 0 && height > 0 && !rgba.is_empty(), "{school}");
        }
        let (rgba, width, height) =
            super::decode_icon(include_bytes!("../assets/icons/sizes/256.png")).unwrap();
        assert_eq!((width, height), (256, 256));
        assert!(!rgba.is_empty());
    }
}
