//! UI wiring: owns the app state, handles every Slint callback and pushes
//! state back into the `Store` global (`refresh`).

use crate::config::{
    self, clean_device_name, combo_label, per_second, split_device, Action, Combo, Config, Device, Hotkey, Hotkeys,
    Macro, MacroMode, MacroStep, Modifier, MouseButton, Output, PadButton, PadLabels, Profile, StickPointer,
    SystemCommand, ThemeMode, LAYER_COUNT,
};
use crate::engine::{self, AppCommand, Engine, EngineEvent, RecEvent};
use crate::system::{self, Autostart};
use crate::{
    ipc, keys, osd, update, AppTray, AppWindow, BindingRow, DeviceItem, KeyCap, MacroItem, MouseBtn, ProfileItem,
    StepRow, Store, Theme,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Order of buttons expected by `MouseDiagram` in ui/mouse.slint.
const MOUSE_ORDER: [&str; 10] = [
    "left",
    "right",
    "middle",
    "wheel_up",
    "wheel_down",
    "back",
    "forward",
    "dpi",
    "wheel_left",
    "wheel_right",
];

/// Shell processes that are not useful as "recent apps".
const IGNORED_APPS: &[&str] = &[
    "explorer.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "applicationframehost.exe",
    "lockapp.exe",
];

// Action kinds (`Store.selected-kind`).
const K_DEFAULT: i32 = 0;
const K_KEY: i32 = 1;
const K_MOUSE: i32 = 2;
const K_MACRO: i32 = 3;
const K_TURBO: i32 = 4;
const K_TEXT: i32 = 5;
const K_LAUNCH: i32 = 6;
const K_MEDIA: i32 = 7;
const K_LAYER: i32 = 8;
const K_SENS: i32 = 9;
const K_TAPHOLD: i32 = 10;
const K_SYSTEM: i32 = 11;
const K_GESTURE: i32 = 12;
const K_GYRO: i32 = 13;
const K_PAD: i32 = 14;
const K_OFF: i32 = 15;

/// Choices for a binding, in kind order.
const KIND_NAMES: [&str; 16] = [
    "Default",
    "Key",
    "Mouse",
    "Macro",
    "Turbo",
    "Text",
    "Launch",
    "Media",
    "Layer",
    "Sensitivity",
    "Tap / hold",
    "System",
    "Gesture",
    "Gyro",
    "Controller",
    "Off",
];

/// Choices for a part of a tap / hold binding, and their kinds.
const SLOT_NAMES: [&str; 13] = [
    "Nothing",
    "Key",
    "Mouse",
    "Macro",
    "Turbo",
    "Text",
    "Launch",
    "Media",
    "Layer",
    "Sensitivity",
    "System",
    "Gyro",
    "Controller",
];
const SLOT_KINDS: [i32; 13] = [
    K_DEFAULT, K_KEY, K_MOUSE, K_MACRO, K_TURBO, K_TEXT, K_LAUNCH, K_MEDIA, K_LAYER, K_SENS, K_SYSTEM, K_GYRO, K_PAD,
];
const SLOT_TITLES: [&str; 4] = ["", "On tap", "On hold", "On double tap"];
const GESTURE_TITLES: [&str; 6] = [
    "",
    "Let go without moving",
    "Swipe up",
    "Swipe down",
    "Swipe left",
    "Swipe right",
];

/// Mouse buttons a combo can hold, after "Nothing" in `combo-mouse-names`.
const COMBO_MOUSE: [MouseButton; 5] = [
    MouseButton::Left,
    MouseButton::Right,
    MouseButton::Middle,
    MouseButton::Back,
    MouseButton::Forward,
];

/// Longest delay / hold accepted from a text field (1 hour).
const MAX_MS: u32 = 3_600_000;

/// Edits closer together than this undo as one step (typing, dragging).
const UNDO_MERGE: Duration = Duration::from_millis(700);
const UNDO_LIMIT: usize = 100;

/// Automatic update checks: how often, and the first one after start.
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const UPDATE_FIRST: Duration = Duration::from_secs(15);

/// What undo restores: bindings, profiles and macros (not settings).
#[derive(Clone)]
struct Snapshot {
    profiles: Vec<Profile>,
    macros: Vec<Macro>,
}

impl Snapshot {
    fn of(cfg: &Config) -> Self {
        Snapshot {
            profiles: cfg.profiles.clone(),
            macros: cfg.macros.clone(),
        }
    }

    fn differs(&self, cfg: &Config) -> bool {
        self.profiles != cfg.profiles || self.macros != cfg.macros
    }
}

struct State {
    cfg: Config,
    view: i32,
    /// Profile shown in the editor.
    profile: usize,
    /// Binding layer shown: 0 base, 1 shift, 2-3.
    layer: usize,
    /// Device whose bindings are shown: 0 all devices, n `cfg.devices[n - 1]`.
    device: usize,
    /// Waiting for a press on a device to add.
    /// (Next field: waiting for a controller press to select it.)
    adding_device: bool,
    detecting: bool,
    /// Capture target whose "Choose from a list" key picker is open ("" none), and its group.
    picker_target: String,
    picker_group: usize,
    /// Selected input id (`key:A`, `mouse:back`, a combo like
    /// `ctrl+mouse:back`) or empty.
    input: String,
    /// Part of a tap / hold binding being edited: 0 the binding itself,
    /// 1 tap, 2 hold, 3 double tap.
    slot: usize,
    /// Action kind chosen in the inspector before a value was picked.
    pending_kind: Option<i32>,
    macro_idx: Option<usize>,
    step: Option<usize>,
    /// Which capture box is waiting for a key.
    capturing: String,
    /// Window capture: a modifier pressed with nothing after it yet.
    ui_lone_mod: Option<&'static str>,
    recording: bool,
    /// Macro the current recording goes into (by name, so selecting or
    /// renaming other macros meanwhile doesn't matter).
    record_target: String,
    /// Waiting to read the pointer position for a "Move to" step.
    picking: bool,
    focus_exe: String,
    focus_self: bool,
    elevation_blocked: bool,
    /// Profile the engine uses now (None while our window is focused), and
    /// the last one it used.
    live: Option<usize>,
    last_live: Option<usize>,
    /// The profile was chosen by hand.
    forced: bool,
    /// Profile last announced on screen.
    osd_profile: Option<usize>,
    /// Announce the profile at the next focus report even if unchanged.
    osd_next_focus: bool,
    recent: Vec<String>,
    autostart: Autostart,
    /// Exe the startup entry opens when it isn't this one (another copy).
    autostart_other: String,
    notice: (String, bool),
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    undo_at: Option<Instant>,
    update: Option<update::Release>,
    update_status: String,
    update_busy: bool,
    update_checked: Option<Instant>,
    window_visible: bool,
    /// The virtual controller, its driver, and installing it.
    vpad: engine::VpadState,
    vpad_driver: bool,
    vpad_installing: bool,
    /// How installing the driver went (shown until the next try).
    vpad_message: String,
}

impl State {
    fn bindings(&self) -> &BTreeMap<String, Action> {
        self.cfg.profiles[self.profile].layer(self.layer)
    }
    fn bindings_mut(&mut self) -> &mut BTreeMap<String, Action> {
        let layer = self.layer;
        self.cfg.profiles[self.profile].layer_mut(layer)
    }
    /// The selected input's binding (the whole tap / hold, if it is one).
    fn binding(&self) -> Option<&Action> {
        self.bindings().get(&self.input)
    }
    fn binding_mut(&mut self) -> Option<&mut Action> {
        let input = self.input.clone();
        self.bindings_mut().get_mut(&input)
    }
    /// The action being edited: the binding, or the tap / hold part.
    fn action(&self) -> Option<&Action> {
        let a = self.binding()?;
        if self.slot == 0 {
            Some(a)
        } else {
            a.slot(self.slot)
        }
    }
    fn action_mut(&mut self) -> Option<&mut Action> {
        let slot = self.slot;
        let a = self.binding_mut()?;
        if slot == 0 {
            Some(a)
        } else {
            a.slot_mut(slot)?.as_deref_mut()
        }
    }
    fn set_action(&mut self, action: Action) {
        if self.input.is_empty() {
            return;
        }
        let slot = self.slot;
        if slot == 0 {
            let input = self.input.clone();
            self.bindings_mut().insert(input, action);
        } else if let Some(s) = self.binding_mut().and_then(|a| a.slot_mut(slot)) {
            *s = Some(Box::new(action));
        }
        self.pending_kind = None;
    }
    /// Back to the default: remove the binding, or empty the tap / hold part.
    fn clear_action(&mut self) {
        let slot = self.slot;
        if slot == 0 {
            let input = self.input.clone();
            self.bindings_mut().remove(&input);
        } else if let Some(s) = self.binding_mut().and_then(|a| a.slot_mut(slot)) {
            *s = None;
        }
        self.pending_kind = None;
    }
    /// A tap / hold or gesture part can only be edited while the binding is one.
    fn fix_slot(&mut self) {
        if self.slot != 0 && !self.binding().is_some_and(|a| a.has_slots()) {
            self.slot = 0;
        }
    }
    /// `@Device` for ids of the device shown, or "" for all devices.
    fn device_suffix(&self) -> String {
        match self.device.checked_sub(1).and_then(|i| self.cfg.devices.get(i)) {
            Some(d) => format!("@{}", d.name),
            None => String::new(),
        }
    }
    fn shown_device(&self) -> Option<&Device> {
        self.device.checked_sub(1).and_then(|i| self.cfg.devices.get(i))
    }
    fn select_input(&mut self, id: &str) {
        self.input = id.to_string();
        self.slot = 0;
        self.pending_kind = None;
    }
    fn current_macro(&mut self) -> Option<&mut Macro> {
        let i = self.macro_idx?;
        self.cfg.macros.get_mut(i)
    }
    fn current_step(&mut self) -> Option<&mut MacroStep> {
        let s = self.step?;
        self.current_macro()?.steps.get_mut(s)
    }
    /// Change the selected combo's keys; its binding moves to the new id.
    fn update_combo(&mut self, f: impl FnOnce(&mut Combo)) {
        let Some(mut c) = Combo::parse(&self.input) else { return };
        f(&mut c);
        if c.mods.is_empty() && c.held.iter().all(|h| *h == c.trigger) {
            return; // a combo has to hold something
        }
        let new_id = c.id();
        if new_id == self.input {
            return;
        }
        let old = std::mem::replace(&mut self.input, new_id.clone());
        if let Some(a) = self.bindings_mut().remove(&old) {
            self.bindings_mut().insert(new_id, a);
        }
    }
    fn push_undo(&mut self, before: Snapshot) {
        let merge = !self.undo.is_empty() && self.undo_at.is_some_and(|t| t.elapsed() < UNDO_MERGE);
        if !merge {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.undo_at = Some(Instant::now());
        self.redo.clear();
    }
    /// The profile in use, or the last one.
    fn shown_live(&self) -> usize {
        self.live
            .or(self.last_live)
            .unwrap_or(0)
            .min(self.cfg.profiles.len() - 1)
    }
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    /// Whether the window has been centred yet (it isn't shown at all when
    /// started with --tray).
    centred: std::cell::Cell<bool>,
    tray: RefCell<Option<AppTray>>,
    engine: Engine,
    st: RefCell<State>,
    timers: RefCell<Vec<slint::Timer>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Run `f` with the app (UI thread only).
pub fn with_app(f: impl FnOnce(&Rc<App>)) {
    let app = APP.with(|a| a.borrow().clone());
    if let Some(app) = app {
        f(&app);
    }
}

/// Ask the running app to show its window (from any thread).
pub fn show_window_from_any_thread() {
    let _ = slint::invoke_from_event_loop(|| with_app(|app| app.show_window()));
}

/// Quit, as asked by `mouse-kb-remap --quit` (from any thread).
pub fn quit_from_any_thread() {
    let _ = slint::invoke_from_event_loop(|| with_app(|app| app.quit()));
}

pub struct Options {
    /// Start in the tray (`--tray`).
    pub start_hidden: bool,
    /// Started by the updater after installing this version.
    pub updated: bool,
}

pub fn run(opts: Options) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    system::stop_raw_keyboard_input();
    repaint_fully_on_activation(&ui);
    let cfg = Config::load();
    ui.global::<Theme>().set_dark(cfg.theme == ThemeMode::Dark);

    // The engine threads report focus changes, captured keys, hotkeys and
    // notices; hop onto the UI thread.
    let engine = Engine::start(&cfg, |ev| {
        let _ = slint::invoke_from_event_loop(move || with_app(|app| app.on_engine_event(ev)));
    });

    let app = Rc::new(App {
        ui: ui.as_weak(),
        centred: std::cell::Cell::new(false),
        tray: RefCell::new(None),
        st: RefCell::new(State {
            cfg,
            view: 0,
            profile: 0,
            layer: 0,
            device: 0,
            adding_device: false,
            detecting: false,
            picker_target: String::new(),
            picker_group: 0,
            input: String::new(),
            slot: 0,
            pending_kind: None,
            macro_idx: None,
            step: None,
            capturing: String::new(),
            ui_lone_mod: None,
            recording: false,
            record_target: String::new(),
            picking: false,
            focus_exe: String::new(),
            focus_self: true,
            elevation_blocked: false,
            live: None,
            last_live: None,
            forced: false,
            osd_profile: None,
            osd_next_focus: false,
            recent: Vec::new(),
            autostart: Autostart::Off,
            autostart_other: String::new(),
            notice: if opts.updated {
                (format!("Updated to version {}.", update::current_version()), false)
            } else {
                (String::new(), false)
            },
            undo: Vec::new(),
            redo: Vec::new(),
            undo_at: None,
            update: None,
            update_status: if opts.updated {
                format!("Updated to version {}.", update::current_version())
            } else {
                String::new()
            },
            update_busy: false,
            update_checked: None,
            window_visible: false,
            vpad: engine::vpad_status(),
            vpad_driver: engine::vpad_driver_present(),
            vpad_installing: false,
            vpad_message: String::new(),
        }),
        engine,
        timers: RefCell::new(Vec::new()),
    });
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    let store = ui.global::<Store>();
    store.set_version(env!("CARGO_PKG_VERSION").into());
    store.set_engine_supported(app.engine.supported());
    store.set_self_elevated(app.engine.elevated());
    store.set_autostart_supported(system::autostart_supported());
    store.set_update_supported(update::supported());
    store.set_config_path(
        Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    store.set_shortcut_names(model(
        keys::SHORTCUTS.iter().map(|(n, _, _)| SharedString::from(*n)).collect(),
    ));
    store.set_mouse_names(model(
        MouseButton::ALL.iter().map(|b| SharedString::from(b.name())).collect(),
    ));
    store.set_media_names(model(
        keys::MEDIA
            .iter()
            .map(|id| SharedString::from(keys::display_name(id)))
            .collect(),
    ));
    store.set_system_names(model(
        SystemCommand::ALL
            .iter()
            .map(|c| SharedString::from(c.name()))
            .collect(),
    ));
    store.set_hotkey_names(model(Hotkeys::NAMES.iter().map(|n| SharedString::from(*n)).collect()));
    store.set_cli_help(
        ipc::FLAGS
            .iter()
            .map(|(flag, _, help)| {
                let flag = if *flag == "--profile" {
                    "--profile \"<name>\""
                } else {
                    flag
                };
                format!("mouse-kb-remap.exe {flag}\n    {help}")
            })
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    );
    wire_callbacks(&ui, &app);

    // Tray icon + close-to-tray.
    let tray = AppTray::new()?;
    tray.on_show_window(|| with_app(|app| app.show_window()));
    tray.on_toggle_enabled(|| with_app(|app| app.edit(|_, st| st.cfg.enabled = !st.cfg.enabled)));
    tray.on_quit(|| with_app(|app| app.quit()));
    *app.tray.borrow_mut() = Some(tray);

    ui.window().on_close_requested(|| {
        with_app(|app| {
            if !app.st.borrow().cfg.close_to_tray {
                app.quit();
            } else {
                app.cancel_capture();
                app.st.borrow_mut().window_visible = false;
            }
        });
        slint::CloseRequestResponse::HideWindow
    });

    // Command line (`--pause`, `--profile <name>`, …): answered on the UI thread.
    ipc::serve(|command| {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|app| {
                let _ = tx.send(app.ipc_command(&command));
            })
        });
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| "error: the app didn't answer".into())
    });

    // Automatic update checks (only when turned on in Settings).
    let first = slint::Timer::default();
    first.start(slint::TimerMode::SingleShot, UPDATE_FIRST, || {
        with_app(|app| app.auto_check_update())
    });
    let hourly = slint::Timer::default();
    hourly.start(slint::TimerMode::Repeated, Duration::from_secs(3600), || {
        with_app(|app| app.auto_check_update())
    });
    app.timers.borrow_mut().extend([first, hourly]);

    if opts.updated {
        app.st.borrow_mut().view = 2;
    }
    app.refresh();
    app.check_autostart();
    if !opts.start_hidden {
        app.show_window();
    }
    slint::run_event_loop_until_quit()?;
    app.engine.shutdown();
    Ok(())
}

/// Centre the window in the work area, shrinking it if the screen is small.
/// (The default position can push the bottom off screen at high scaling.)
/// Repaint the whole window when it's activated, deactivated, uncovered or
/// shown. The software renderer only redraws what changed in the UI, so a
/// region Windows wants repainted (after alt-tab, or the first show from the
/// tray) could otherwise stay unpainted and show what's behind the window.
/// See `repaint-tick` in ui/app.slint.
fn repaint_fully_on_activation(ui: &AppWindow) {
    use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        if matches!(
            event,
            WindowEvent::Focused(_) | WindowEvent::Occluded(false) | WindowEvent::ThemeChanged(_)
        ) {
            if let Some(ui) = weak.upgrade() {
                ui.set_repaint_tick(!ui.get_repaint_tick());
                ui.window().request_redraw();
            }
        }
        EventResult::Propagate
    });
}

fn center_window(ui: &AppWindow) {
    let Some((x, y, w, h)) = system::work_area() else {
        return;
    };
    let window = ui.window();
    let size = window.size();
    let (ww, wh) = (size.width.min(w as u32), size.height.min(h as u32));
    if (ww, wh) != (size.width, size.height) {
        window.set_size(slint::PhysicalSize::new(ww, wh));
    }
    window.set_position(slint::PhysicalPosition::new(
        x + (w - ww as i32) / 2,
        y + (h - wh as i32) / 2,
    ));
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn parse_ms(text: &str) -> Option<u32> {
    let t = text.trim().trim_end_matches("ms").trim();
    t.parse::<u64>().ok().map(|v| v.min(MAX_MS as u64) as u32)
}

fn parse_coord(text: &str) -> Option<i32> {
    text.trim()
        .replace('−', "-")
        .trim_start_matches('+')
        .parse::<i64>()
        .ok()
        .map(|v| v.clamp(-100_000, 100_000) as i32)
}

fn kind_of(action: &Action) -> i32 {
    match action {
        Action::Key { key, .. } if keys::MEDIA.contains(&key.as_str()) => K_MEDIA,
        Action::Key { .. } => K_KEY,
        Action::Mouse { .. } => K_MOUSE,
        Action::Macro { .. } => K_MACRO,
        Action::Turbo { .. } => K_TURBO,
        Action::Text { .. } => K_TEXT,
        Action::Launch { .. } => K_LAUNCH,
        Action::Shift | Action::Layer { .. } => K_LAYER,
        Action::Sensitivity { .. } => K_SENS,
        Action::TapHold { .. } => K_TAPHOLD,
        Action::System { .. } => K_SYSTEM,
        Action::Gesture { .. } => K_GESTURE,
        Action::Gyro { .. } => K_GYRO,
        Action::Pad { .. } => K_PAD,
        Action::Disabled => K_OFF,
    }
}

fn step_kind(step: &MacroStep) -> i32 {
    match step {
        MacroStep::KeyTap { .. } => 0,
        MacroStep::KeyDown { .. } => 1,
        MacroStep::KeyUp { .. } => 2,
        MacroStep::MouseClick { .. } => 3,
        MacroStep::MouseDown { .. } => 4,
        MacroStep::MouseUp { .. } => 5,
        MacroStep::Delay { .. } => 6,
        MacroStep::Text { .. } => 7,
        MacroStep::MouseMove { absolute: false, .. } => 8,
        MacroStep::MouseMove { absolute: true, .. } => 9,
    }
}

fn step_key(step: &MacroStep) -> Option<&str> {
    match step {
        MacroStep::KeyTap { key, .. } | MacroStep::KeyDown { key } | MacroStep::KeyUp { key } => Some(key),
        _ => None,
    }
}

fn step_button(step: &MacroStep) -> Option<MouseButton> {
    match step {
        MacroStep::MouseClick { button, .. } | MacroStep::MouseDown { button } | MacroStep::MouseUp { button } => {
            Some(*button)
        }
        _ => None,
    }
}

fn step_ms(step: &MacroStep) -> Option<u32> {
    match step {
        MacroStep::KeyTap { hold_ms, .. } | MacroStep::MouseClick { hold_ms, .. } => Some(*hold_ms),
        MacroStep::Delay { ms } => Some(*ms),
        _ => None,
    }
}

fn step_xy(step: &MacroStep) -> Option<(i32, i32)> {
    match step {
        MacroStep::MouseMove { x, y, .. } => Some((*x, *y)),
        _ => None,
    }
}

fn make_step(kind: i32, key: String, button: MouseButton, ms: Option<u32>, xy: Option<(i32, i32)>) -> MacroStep {
    match kind {
        0 => MacroStep::KeyTap {
            key,
            hold_ms: ms.unwrap_or(20),
        },
        1 => MacroStep::KeyDown { key },
        2 => MacroStep::KeyUp { key },
        3 => MacroStep::MouseClick {
            button,
            hold_ms: ms.unwrap_or(20),
        },
        4 => MacroStep::MouseDown { button },
        5 => MacroStep::MouseUp { button },
        6 => MacroStep::Delay { ms: ms.unwrap_or(50) },
        8 => {
            let (x, y) = xy.unwrap_or((0, 0));
            MacroStep::MouseMove { x, y, absolute: false }
        }
        9 => {
            let (x, y) = xy.or_else(system::cursor_pos).unwrap_or((0, 0));
            MacroStep::MouseMove { x, y, absolute: true }
        }
        _ => MacroStep::Text { text: String::new() },
    }
}

/// Plain input ids look like `key:A`, `key:F1`, `mouse:back`, `pad:a`, and
/// may be limited to a device: `mouse:back@Trackball` → "Back (Trackball)".
fn plain_label(input: &str) -> String {
    let (input, device) = split_device(input);
    let label = if let Some(k) = input.strip_prefix("key:") {
        keys::display_name(k)
    } else if let Some(m) = input.strip_prefix("mouse:") {
        MouseButton::from_id(m)
            .map(|b| b.name().to_string())
            .unwrap_or_else(|| m.to_string())
    } else if let Some(p) = input.strip_prefix("pad:") {
        PadButton::from_id(p).map(pad_name).unwrap_or_else(|| p.to_string())
    } else {
        input.to_string()
    };
    match device {
        Some(d) => format!("{label} ({d})"),
        None => label,
    }
}

/// Whether keys can have bindings for one keyboard. Windows can't tell
/// keyboards apart (see engine/win_devices.rs); Linux reads each one.
const KEYBOARDS_PER_DEVICE: bool = cfg!(target_os = "linux");

thread_local! {
    /// Controller button names in use (set by `refresh`), for labels made
    /// outside it.
    static PAD_STYLE: std::cell::Cell<PadLabels> = const { std::cell::Cell::new(PadLabels::Xbox) };
}

/// A controller input's name in the chosen style: "Cross ×" for the bottom
/// face button with PlayStation names, Xbox's descriptive names otherwise.
fn pad_name(b: PadButton) -> String {
    let style = PAD_STYLE.with(|s| s.get());
    let short = short_pad_label(b, style);
    if style != PadLabels::Xbox && short != short_pad_label(b, PadLabels::Xbox) {
        short
    } else {
        b.name()
    }
}

/// What's printed on a part of the controller drawing.
fn pad_glyph(b: PadButton, style: PadLabels) -> &'static str {
    use PadLabels::{Nintendo, PlayStation};
    match (b, style) {
        (PadButton::A, PlayStation) => "×",
        (PadButton::B, PlayStation) => "○",
        (PadButton::X, PlayStation) => "□",
        (PadButton::Y, PlayStation) => "△",
        (PadButton::A, Nintendo) => "B",
        (PadButton::B, Nintendo) => "A",
        (PadButton::X, Nintendo) => "Y",
        (PadButton::Y, Nintendo) => "X",
        (PadButton::A, _) => "A",
        (PadButton::B, _) => "B",
        (PadButton::X, _) => "X",
        (PadButton::Y, _) => "Y",
        (PadButton::Lb, PlayStation) => "L1",
        (PadButton::Rb, PlayStation) => "R1",
        (PadButton::Lt, PlayStation) => "L2",
        (PadButton::Rt, PlayStation) => "R2",
        (PadButton::Ls, PlayStation) => "L3",
        (PadButton::Rs, PlayStation) => "R3",
        (PadButton::Lb, Nintendo) => "L",
        (PadButton::Rb, Nintendo) => "R",
        (PadButton::Lt, Nintendo) => "ZL",
        (PadButton::Rt, Nintendo) => "ZR",
        (PadButton::Lb, _) => "LB",
        (PadButton::Rb, _) => "RB",
        (PadButton::Lt, _) => "LT",
        (PadButton::Rt, _) => "RT",
        (PadButton::Ls, _) => "LS",
        (PadButton::Rs, _) => "RS",
        (PadButton::Guide, PlayStation) => "PS",
        (PadButton::Guide, Nintendo) => "⌂",
        (PadButton::Guide, _) => "G",
        (PadButton::Touchpad, _) => "touchpad",
        _ => "",
    }
}

/// Short names for the controller drawing, in the chosen button style.
/// Positions stay the same: Xbox A, PlayStation Cross and Nintendo B are all
/// the bottom face button.
/// A virtual controller output in the picker: directions as arrows, so
/// they fit three to a row ("Right stick ↓").
fn output_grid_name(b: PadButton) -> String {
    let name = b.name();
    for (word, arrow) in [(" up", " ↑"), (" down", " ↓"), (" left", " ←"), (" right", " →")] {
        if let Some(rest) = name.strip_suffix(word) {
            return format!("{rest}{arrow}");
        }
    }
    name
}

/// Position of a virtual controller output in `PadButton::OUTPUTS`.
fn output_index(b: PadButton) -> i32 {
    PadButton::OUTPUTS.iter().position(|o| *o == b).map_or(-1, |i| i as i32)
}

/// The virtual controller box: (`Store.vpad-state`, text).
fn vpad_display(st: &State) -> (i32, String) {
    let note = if st.vpad_message.is_empty() {
        String::new()
    } else {
        format!(" {}", st.vpad_message)
    };
    if !engine::VPAD_SUPPORTED {
        return (3, "The virtual controller is only available on Windows for now.".into());
    }
    if st.vpad_installing {
        return (
            4,
            format!(
                "Installing {}: allow it in Windows' admin prompt, then follow the installer's steps.",
                crate::driver::NAME
            ),
        );
    }
    if !st.vpad_driver {
        return (
            0,
            format!(
                "Needs the free ViGEmBus driver, which isn't installed.{note} The app downloads the official \
                 installer ({}, signed by Nefarius Software Solutions), checks that it's exactly the expected \
                 file and runs it after Windows' admin prompt. From {}.",
                crate::driver::NAME,
                crate::driver::PROJECT_URL
            ),
        );
    }
    match &st.vpad {
        engine::VpadState::Plugged { slot } => (
            2,
            match slot {
                Some(n) => format!("Virtual controller plugged in, as controller {n}.{note}"),
                None => format!("Virtual controller plugged in.{note}"),
            },
        ),
        engine::VpadState::Failed(e) => (3, format!("The virtual controller couldn't be plugged in: {e}.")),
        _ => (
            1,
            format!("ViGEmBus is installed.{note} The virtual controller is plugged in while a profile that uses it is active."),
        ),
    }
}

fn short_pad_label(b: PadButton, style: PadLabels) -> String {
    use PadLabels::{Nintendo, PlayStation};
    let s = match (b, style) {
        (PadButton::A, PlayStation) => "Cross ×",
        (PadButton::B, PlayStation) => "Circle ○",
        (PadButton::X, PlayStation) => "Square □",
        (PadButton::Y, PlayStation) => "Triangle △",
        (PadButton::Lb, PlayStation) => "L1",
        (PadButton::Rb, PlayStation) => "R1",
        (PadButton::Lt, PlayStation) => "L2",
        (PadButton::Rt, PlayStation) => "R2",
        (PadButton::Back, PlayStation) => "Create",
        (PadButton::Start, PlayStation) => "Options",
        (PadButton::Ls, PlayStation) => "L3",
        (PadButton::Rs, PlayStation) => "R3",
        (PadButton::Guide, PlayStation) => "PS",
        (PadButton::A, Nintendo) => "B",
        (PadButton::B, Nintendo) => "A",
        (PadButton::X, Nintendo) => "Y",
        (PadButton::Y, Nintendo) => "X",
        (PadButton::Lb, Nintendo) => "L",
        (PadButton::Rb, Nintendo) => "R",
        (PadButton::Lt, Nintendo) => "ZL",
        (PadButton::Rt, Nintendo) => "ZR",
        (PadButton::Back, Nintendo) => "Minus −",
        (PadButton::Start, Nintendo) => "Plus +",
        (PadButton::Guide, Nintendo) => "Home",
        (PadButton::Share, Nintendo) => "Capture",
        (PadButton::A, _) => "A",
        (PadButton::B, _) => "B",
        (PadButton::X, _) => "X",
        (PadButton::Y, _) => "Y",
        (PadButton::Lb, _) => "LB",
        (PadButton::Rb, _) => "RB",
        (PadButton::Lt, _) => "LT",
        (PadButton::Rt, _) => "RT",
        (PadButton::Back, _) => "View",
        (PadButton::Start, _) => "Menu",
        (PadButton::Ls, _) => "LS click",
        (PadButton::Rs, _) => "RS click",
        (PadButton::Up, _) => "D-pad ↑",
        (PadButton::Down, _) => "D-pad ↓",
        (PadButton::Left, _) => "D-pad ←",
        (PadButton::Right, _) => "D-pad →",
        (PadButton::LsUp, _) => "LS ↑",
        (PadButton::LsDown, _) => "LS ↓",
        (PadButton::LsLeft, _) => "LS ←",
        (PadButton::LsRight, _) => "LS →",
        (PadButton::RsUp, _) => "RS ↑",
        (PadButton::RsDown, _) => "RS ↓",
        (PadButton::RsLeft, _) => "RS ←",
        (PadButton::RsRight, _) => "RS →",
        (PadButton::Guide, _) => "Guide",
        (PadButton::Share, _) => "Share",
        (PadButton::Touchpad, _) => "Touchpad",
        (PadButton::Mic, _) => "Mic",
        (PadButton::P1, _) => "Extra 1",
        (PadButton::P2, _) => "Extra 2",
        (PadButton::P3, _) => "Extra 3",
        (PadButton::P4, _) => "Extra 4",
        (PadButton::LeftPad, _) => "L pad click",
        (PadButton::Button(n), _) => return format!("Button {n}"),
    };
    s.to_string()
}

/// Groups of the "Choose from a list" key picker.
const PICKER_GROUPS: [&str; 9] = [
    "Letters",
    "Numbers",
    "F1–F24",
    "Modifiers",
    "Editing",
    "Navigation",
    "Symbols",
    "Number pad",
    "Media",
];

/// Which picker group a key is listed under.
fn picker_group_of(k: &keys::KeyDef) -> usize {
    if k.group == keys::Group::Numpad {
        return 7;
    }
    match k.vk {
        0x41..=0x5A => 0,
        0x30..=0x39 => 1,
        0x70..=0x87 => 2,
        0x14 | 0x5D => 3,
        _ if keys::modifier_of(k.vk).is_some() => 3,
        0x21..=0x28 => 5,
        0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 => 6,
        _ if keys::MEDIA.contains(&k.id) || (0xA6..=0xB7).contains(&k.vk) => 8,
        _ => 4,
    }
}

/// Keys of a picker group: letters A-Z and digits 0-9 in order, the rest
/// in keyboard order.
fn picker_keys(group: usize) -> Vec<&'static keys::KeyDef> {
    let mut list: Vec<_> = keys::KEYS.iter().filter(|k| picker_group_of(k) == group).collect();
    if group <= 1 {
        list.sort_by_key(|k| k.vk);
    }
    list
}

/// The key a capture target holds now (`binding`, `turbo`, `step`, `combo`).
fn picker_current(st: &State, target: &str) -> Option<String> {
    match target {
        "binding" => match st.action() {
            Some(Action::Key { key, .. }) => Some(key.clone()),
            _ => None,
        },
        "turbo" => match st.action() {
            Some(Action::Turbo {
                output: Output::Key { key, .. },
                ..
            }) => Some(key.clone()),
            _ => None,
        },
        "step" => st
            .macro_idx
            .and_then(|m| st.cfg.macros.get(m))
            .and_then(|m| st.step.and_then(|i| m.steps.get(i)))
            .and_then(|s| step_key(s).map(str::to_string)),
        "combo" => {
            Combo::parse(&st.input).and_then(|c| c.held.iter().find_map(|h| h.strip_prefix("key:").map(str::to_string)))
        }
        _ => None,
    }
}

/// The recognisable part of a device id, for the device list:
/// "VID_046D PID_C539", "slot 1", "046d:c539".
fn device_id_summary(id: &str) -> String {
    if let Some(slot) = id.strip_prefix("xinput:") {
        return format!("slot {slot}");
    }
    if let Some(rest) = id.strip_prefix("evdev:") {
        return rest.splitn(3, ':').take(2).collect::<Vec<_>>().join(":");
    }
    let upper = id.to_ascii_uppercase();
    let part = |key: &str| {
        upper
            .find(key)
            .map(|i| upper[i..].chars().take(key.len() + 4).collect::<String>())
    };
    match (part("VID_"), part("PID_")) {
        (Some(v), Some(p)) => format!("{v} {p}"),
        _ => id.chars().take(40).collect(),
    }
}

/// "Back", or for a combo "Ctrl + Back".
fn input_label(input: &str) -> String {
    match Combo::parse(input) {
        Some(c) => {
            let mut mods = c.mods.clone();
            mods.sort();
            let mut parts: Vec<String> = mods.iter().map(|m| m.name().to_string()).collect();
            parts.extend(c.held.iter().map(|h| plain_label(h)));
            parts.push(plain_label(&c.trigger));
            parts.join(" + ")
        }
        None => plain_label(input),
    }
}

fn input_detail(input: &str) -> String {
    let (base, device) = split_device(input);
    if let Some(d) = device {
        let what = input_detail(base);
        return if what.is_empty() {
            format!("Only on {d}")
        } else {
            format!("{what} · only on {d}")
        };
    }
    let input = base;
    if Combo::parse(input).is_some() {
        "Combo: hold the first, then press the last".to_string()
    } else if let Some(p) = input.strip_prefix("pad:") {
        PadButton::from_id(p)
            .map(|b| b.detail().to_string())
            .unwrap_or_default()
    } else if let Some(k) = input.strip_prefix("key:") {
        match keys::find(k).map(|k| k.group) {
            Some(keys::Group::Extra) => "Keyboard key (not on the drawing)".to_string(),
            Some(keys::Group::Numpad) => "Number pad key".to_string(),
            _ => "Keyboard key".to_string(),
        }
    } else if let Some(m) = input.strip_prefix("mouse:") {
        MouseButton::from_id(m)
            .map(|b| b.detail().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

/// "the shift layer", "layer 2".
fn layer_phrase(layer: usize) -> String {
    match layer {
        1 => "the shift layer".to_string(),
        n => format!("layer {n}"),
    }
}

/// What an unbound input in this profile/layer does instead, as a sentence.
fn inherited(cfg: &Config, profile: usize, layer: usize, input: &str) -> Option<String> {
    let def = &cfg.profiles[0];
    let p = &cfg.profiles[profile];
    if let (base, Some(device)) = split_device(input) {
        if profile > 0 {
            if let Some(a) = def.layer(layer).get(input) {
                return Some(format!("Inherits from the Default profile: {}.", a.label()));
            }
        }
        if let Some(a) = p.layer(layer).get(base) {
            return Some(format!(
                "Nothing just for {device}, so the binding for all devices applies: {}.",
                a.label()
            ));
        }
        return Some(match inherited(cfg, profile, layer, base) {
            Some(s) => format!("Nothing just for {device}. {s}"),
            None => format!("Nothing just for {device}: the binding for all devices applies (none)."),
        });
    }
    if layer > 0 {
        let name = layer_phrase(layer);
        if profile > 0 {
            if let Some(a) = def.layer(layer).get(input) {
                return Some(format!("Uses the Default profile's {name}: {}.", a.label()));
            }
        }
        if let Some(a) = p.bindings.get(input) {
            return Some(format!(
                "Nothing on {name}, so the base binding applies: {}.",
                a.label()
            ));
        }
        if profile > 0 {
            if let Some(a) = def.bindings.get(input) {
                return Some(format!(
                    "Nothing on {name}, so Default's binding applies: {}.",
                    a.label()
                ));
            }
        }
        None
    } else if profile > 0 {
        def.bindings
            .get(input)
            .map(|a| format!("Inherits from the Default profile: {}.", a.label()))
    } else {
        None
    }
}

/// Short form of `inherited` for callouts.
fn inherited_short(cfg: &Config, profile: usize, layer: usize, input: &str) -> Option<String> {
    let def = &cfg.profiles[0];
    let p = &cfg.profiles[profile];
    if let (base, Some(_)) = split_device(input) {
        if profile > 0 {
            if let Some(a) = def.layer(layer).get(input) {
                return Some(format!("{} (Default)", a.label()));
            }
        }
        return p
            .layer(layer)
            .get(base)
            .map(|a| a.label())
            .or_else(|| inherited_short(cfg, profile, layer, base))
            .map(|s| {
                format!(
                    "{} (all devices)",
                    s.trim_end_matches(" (base)").trim_end_matches(" (Default)")
                )
            });
    }
    if layer > 0 {
        let a = (profile > 0)
            .then(|| def.layer(layer).get(input))
            .flatten()
            .or_else(|| p.bindings.get(input))
            .or_else(|| (profile > 0).then(|| def.bindings.get(input)).flatten())?;
        Some(format!("{} (base)", a.label()))
    } else if profile > 0 {
        def.bindings.get(input).map(|a| format!("{} (Default)", a.label()))
    } else {
        None
    }
}

/// "SHIFT LAYER · hold Back to use" for the layer banner.
fn layer_banner(cfg: &Config, profile: usize, layer: usize) -> String {
    let own = &cfg.profiles[profile].bindings;
    let inherited = cfg.profiles[0]
        .bindings
        .iter()
        .filter(|(i, _)| profile > 0 && !own.contains_key(*i));
    let mut hold = Vec::new();
    let mut toggle = Vec::new();
    for (input, action) in own.iter().chain(inherited) {
        let mut top = true;
        action.walk(&mut |a| {
            if let Action::Layer { layer: l, toggle: t } = a {
                if *l as usize == layer {
                    let label = input_label(input);
                    // Inside a tap / hold: the hold part holds, the tap part toggles.
                    if *t {
                        toggle.push(label);
                    } else {
                        hold.push(if top { label } else { format!("{label} (hold)") });
                    }
                }
            }
            top = false;
        });
    }
    let title = match layer {
        1 => "SHIFT LAYER".to_string(),
        n => format!("LAYER {n}"),
    };
    let how = match (hold.is_empty(), toggle.is_empty()) {
        (true, true) => "no input switches to it yet (use the Layer action)".to_string(),
        (false, true) => format!("hold {} to use", hold.join(" or ")),
        (true, false) => format!("press {} to switch on / off", toggle.join(" or ")),
        (false, false) => format!(
            "hold {}, or press {} to switch on / off",
            hold.join(" or "),
            toggle.join(" or ")
        ),
    };
    format!("{title} · {how}")
}

impl App {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// Change config: then save, push to the engine and redraw. Changes to
    /// profiles and macros can be undone.
    fn edit(&self, f: impl FnOnce(&App, &mut State)) {
        {
            let mut st = self.st.borrow_mut();
            let before = Snapshot::of(&st.cfg);
            f(self, &mut st);
            st.cfg.profiles.iter_mut().for_each(Profile::tidy);
            st.fix_slot();
            if before.differs(&st.cfg) {
                st.push_undo(before);
            }
            st.cfg.save();
            self.engine.apply(&st.cfg);
        }
        self.refresh();
    }

    /// Change only what's shown, then redraw.
    fn nav(&self, f: impl FnOnce(&App, &mut State)) {
        {
            let mut st = self.st.borrow_mut();
            f(self, &mut st);
            st.fix_slot();
        }
        self.refresh();
    }

    fn notice(&self, text: impl Into<String>, error: bool) {
        self.st.borrow_mut().notice = (text.into(), error);
        self.refresh();
    }

    /// An on-screen notification, if they're turned on.
    fn osd(&self, text: &str) {
        if self.st.borrow().cfg.notifications {
            osd::show(text);
        }
    }

    pub fn show_window(&self) {
        let Some(ui) = self.ui() else { return };
        let _ = ui.show();
        ui.window().set_minimized(false);
        self.st.borrow_mut().window_visible = true;
        // Paint everything once the window is really on screen.
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_repaint_tick(!ui.get_repaint_tick());
                ui.window().request_redraw();
            }
        });
        if !self.centred.replace(true) {
            // The real size is only known once the window exists.
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(30), move || {
                if let Some(ui) = weak.upgrade() {
                    center_window(&ui);
                }
            });
        }
    }

    fn quit(&self) {
        self.cancel_capture();
        if self.st.borrow().recording {
            self.engine.stop_recording();
        }
        let _ = slint::quit_event_loop();
    }

    fn cancel_capture(&self) {
        self.engine.cancel_capture();
        let changed = {
            let mut st = self.st.borrow_mut();
            !std::mem::take(&mut st.capturing).is_empty()
        };
        if changed {
            self.refresh();
        }
    }

    fn start_capture(&self, target: &str) {
        if self.st.borrow().recording {
            return;
        }
        // Without a selected input there's nothing to bind ("detect" picks one).
        if (target == "binding" || target == "combo") && self.st.borrow().input.is_empty() {
            return;
        }
        // Engine capture where possible; the window's FocusScope otherwise.
        let _ = self.engine.start_capture();
        self.nav(|_, st| st.capturing = target.to_string());
    }

    fn on_engine_event(&self, ev: EngineEvent) {
        match ev {
            EngineEvent::Focus(info) => {
                self.nav(|_, st| {
                    st.focus_self = info.is_self;
                    if !info.is_self {
                        // The engine cancels capture when focus leaves us.
                        st.capturing.clear();
                    }
                    if !info.is_self && !info.exe.is_empty() {
                        let exe = info.exe.clone();
                        st.elevation_blocked = info.elevation_blocked;
                        if !IGNORED_APPS.contains(&exe.to_ascii_lowercase().as_str()) {
                            st.recent.retain(|a| !a.eq_ignore_ascii_case(&exe));
                            st.recent.insert(0, exe.clone());
                            st.recent.truncate(6);
                        }
                        st.focus_exe = exe;
                    }
                    st.live = info.profile;
                    st.forced = info.forced;
                    if info.profile.is_some() {
                        st.last_live = info.profile;
                    }
                });
                // Announce profile switches (not the first one at start).
                if let Some(p) = info.profile {
                    let text = {
                        let mut st = self.st.borrow_mut();
                        let announce =
                            std::mem::take(&mut st.osd_next_focus) || st.osd_profile.is_some_and(|last| last != p);
                        st.osd_profile = Some(p);
                        let name = st.cfg.profiles.get(p).map(|p| p.name.clone()).unwrap_or_default();
                        announce.then(|| {
                            if info.forced {
                                format!("Profile: {name} (chosen by hand)")
                            } else {
                                format!("Profile: {name}")
                            }
                        })
                    };
                    if let Some(text) = text {
                        self.osd(&text);
                    }
                }
            }
            EngineEvent::Captured { vk, mods } => {
                if let Some(k) = keys::find_by_vk(vk) {
                    self.captured(k.id, mods);
                } else {
                    // A key we don't know: listen for another one.
                    let _ = self.engine.start_capture();
                }
            }
            EngineEvent::Command(cmd) => self.command(cmd),
            EngineEvent::Notice(text) => self.osd(&text),
            EngineEvent::DeviceCaptured { id, name } => self.device_captured(id, name),
            EngineEvent::Vpad(state) => self.nav(|_, st| {
                match state {
                    engine::VpadState::Plugged { .. } => st.vpad_driver = true,
                    engine::VpadState::NoDriver => st.vpad_driver = false,
                    _ => {}
                }
                st.vpad = state;
            }),
            EngineEvent::PadDetected { input } => {
                if let Some(ui) = self.ui() {
                    let s = ui.global::<Store>();
                    s.set_device_tab(2);
                    s.set_inspector_tab(0);
                }
                self.nav(|_, st| {
                    st.detecting = false;
                    let id = format!("{input}{}", st.device_suffix());
                    st.select_input(&id);
                });
            }
        }
    }

    /// "Add a device": wait for a press on it, or stop waiting.
    fn toggle_add_device(&self) {
        let on = !self.st.borrow().adding_device;
        self.engine.capture_device(on);
        self.nav(|_, st| {
            st.adding_device = on;
            st.notice = (String::new(), false);
        });
    }

    fn device_captured(&self, id: String, name: String) {
        if !self.st.borrow().adding_device {
            return;
        }
        let existing = self
            .st
            .borrow()
            .cfg
            .devices
            .iter()
            .find(|d| d.id.eq_ignore_ascii_case(&id))
            .map(|d| d.name.clone());
        if let Some(existing) = existing {
            self.nav(|_, st| {
                st.adding_device = false;
                st.notice = (format!("That one is already added, as \"{existing}\"."), false);
            });
            return;
        }
        self.edit(|_, st| {
            st.adding_device = false;
            let name = st.cfg.unique_device_name(&name);
            st.cfg.devices.push(Device {
                name: name.clone(),
                id: id.clone(),
            });
            st.notice = (
                format!("Added \"{name}\". Rename it here, then pick it under \"Bindings for\" in a profile."),
                false,
            );
        });
    }

    /// An app hotkey or a "Next profile" / "Automatic profile" binding.
    fn command(&self, cmd: AppCommand) {
        match cmd {
            AppCommand::TogglePause => {
                let on = !self.st.borrow().cfg.enabled;
                self.set_enabled(on);
            }
            AppCommand::NextProfile => {
                self.next_profile();
            }
            AppCommand::AutoProfile => self.force_profile(None),
        }
    }

    fn set_enabled(&self, on: bool) {
        if self.st.borrow().cfg.enabled != on {
            self.edit(|_, st| st.cfg.enabled = on);
            self.osd(if on { "Remapping on" } else { "Remapping paused" });
        }
    }

    fn force_profile(&self, index: Option<usize>) {
        self.st.borrow_mut().osd_next_focus = true;
        self.engine.force_profile(index);
    }

    /// Use the profile after the one in use. Returns its name.
    fn next_profile(&self) -> String {
        let (next, name) = {
            let st = self.st.borrow();
            let next = (st.shown_live() + 1) % st.cfg.profiles.len();
            (next, st.cfg.profiles[next].name.clone())
        };
        self.force_profile(Some(next));
        name
    }

    /// A command from `mouse-kb-remap --pause` etc. Answers start with "ok"
    /// or "error".
    fn ipc_command(&self, command: &str) -> String {
        let (verb, arg) = command.split_once(' ').unwrap_or((command, ""));
        let arg = arg.trim().trim_matches('"');
        let enabled = self.st.borrow().cfg.enabled;
        let state = |on: bool| if on { "on" } else { "paused" };
        match verb {
            "pause" | "resume" | "toggle-pause" => {
                let on = match verb {
                    "pause" => false,
                    "resume" => true,
                    _ => !enabled,
                };
                self.set_enabled(on);
                format!("ok: remapping {}", state(on))
            }
            "next-profile" => format!("ok: profile {}", self.next_profile()),
            "auto-profile" => {
                self.force_profile(None);
                "ok: choosing the profile by the focused app".into()
            }
            "profile" => {
                let found = {
                    let st = self.st.borrow();
                    st.cfg
                        .profiles
                        .iter()
                        .position(|p| p.name.eq_ignore_ascii_case(arg))
                        .map(|i| (i, st.cfg.profiles[i].name.clone()))
                        .ok_or_else(|| {
                            let names: Vec<&str> = st.cfg.profiles.iter().map(|p| p.name.as_str()).collect();
                            format!("error: no profile named \"{arg}\" (there are: {})", names.join(", "))
                        })
                };
                match found {
                    Ok((i, name)) => {
                        self.force_profile(Some(i));
                        format!("ok: profile {name}")
                    }
                    Err(e) => e,
                }
            }
            "status" => {
                let st = self.st.borrow();
                format!(
                    "ok: remapping {}; profile {} ({}); focused app {}",
                    state(enabled),
                    st.cfg.profiles[st.shown_live()].name,
                    if st.forced { "chosen by hand" } else { "automatic" },
                    if st.focus_exe.is_empty() {
                        "unknown"
                    } else {
                        &st.focus_exe
                    }
                )
            }
            "show" => {
                self.show_window();
                "ok".into()
            }
            _ => format!("error: unknown command \"{verb}\""),
        }
    }

    /// A key arrived for the capture box that was waiting.
    fn captured(&self, key: &str, mods: u8) {
        let target = std::mem::take(&mut self.st.borrow_mut().capturing);
        let key = key.to_string();
        match target.as_str() {
            "binding" => self.edit(|_, st| {
                st.set_action(Action::Key {
                    key,
                    mods: Modifier::from_bits(mods),
                })
            }),
            "turbo" => self.edit(|_, st| {
                if let Some(Action::Turbo { output, .. }) = st.action_mut() {
                    *output = Output::Key {
                        key,
                        mods: Modifier::from_bits(mods),
                    };
                }
            }),
            "step" => self.edit(|_, st| {
                if let Some(
                    MacroStep::KeyTap { key: k, .. } | MacroStep::KeyDown { key: k } | MacroStep::KeyUp { key: k },
                ) = st.current_step()
                {
                    *k = key
                }
            }),
            "combo" => self.edit(|_, st| {
                let modifier = keys::find(&key).and_then(|k| keys::modifier_of(k.vk));
                st.update_combo(|c| match modifier {
                    // A modifier key counts as Ctrl / Shift / … (either side).
                    Some(m) => {
                        if !c.mods.contains(&m) {
                            c.mods.push(m);
                        }
                    }
                    None => c.held = vec![format!("key:{key}")],
                });
            }),
            "detect" => {
                if let Some(ui) = self.ui() {
                    let store = ui.global::<Store>();
                    if keys::find(&key).is_some_and(|k| k.group != keys::Group::Extra) {
                        store.set_device_tab(1);
                    }
                    store.set_inspector_tab(0);
                }
                self.nav(|_, st| {
                    st.view = 0;
                    st.select_input(&format!("key:{key}"));
                    if keys::find(&key).is_some_and(|k| k.group == keys::Group::Numpad) {
                        st.cfg.full_keyboard = true;
                    }
                });
            }
            t if t.starts_with("hotkey") => {
                let i: usize = t["hotkey".len()..].parse().unwrap_or(0);
                self.edit(|_, st| {
                    *st.cfg.hotkeys.slot_mut(i) = Some(Hotkey {
                        key,
                        mods: Modifier::from_bits(mods),
                    })
                });
            }
            _ => self.refresh(),
        }
    }

    fn undo(&self, redo: bool) {
        {
            let mut st = self.st.borrow_mut();
            let Some(snap) = (if redo { st.redo.pop() } else { st.undo.pop() }) else {
                return;
            };
            let current = Snapshot::of(&st.cfg);
            if redo {
                st.undo.push(current);
            } else {
                st.redo.push(current);
            }
            st.cfg.profiles = snap.profiles;
            st.cfg.macros = snap.macros;
            st.undo_at = None;
            // Keep the selection valid.
            st.profile = st.profile.min(st.cfg.profiles.len() - 1);
            st.macro_idx = st.macro_idx.filter(|i| *i < st.cfg.macros.len());
            if st.view == 1 && st.macro_idx.is_none() {
                st.view = 0;
            }
            st.step = None;
            st.pending_kind = None;
            st.fix_slot();
            st.cfg.save();
            self.engine.apply(&st.cfg);
        }
        self.refresh();
    }

    // ---- macros ----------------------------------------------------------------

    fn toggle_record(&self) {
        let recording = self.st.borrow().recording;
        if !recording {
            self.cancel_capture();
            if self.st.borrow().macro_idx.is_none() || !self.engine.start_recording() {
                return;
            }
            self.nav(|_, st| {
                st.recording = true;
                st.record_target = st
                    .macro_idx
                    .and_then(|i| st.cfg.macros.get(i))
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
            });
            return;
        }
        let mut events = self.engine.stop_recording();
        // Drop the click on "Stop recording" itself.
        if matches!(
            events.last(),
            Some((
                _,
                RecEvent::Button {
                    button: MouseButton::Left,
                    down: false
                }
            ))
        ) {
            events.pop();
            if matches!(
                events.last(),
                Some((
                    _,
                    RecEvent::Button {
                        button: MouseButton::Left,
                        down: true
                    }
                ))
            ) {
                events.pop();
            }
        }
        let steps = engine::recording_to_steps(&events);
        self.edit(|_, st| {
            st.recording = false;
            let target = std::mem::take(&mut st.record_target);
            if let Some(m) = st.cfg.macros.iter_mut().find(|m| m.name == target) {
                if !m.steps.is_empty() && !steps.is_empty() {
                    m.steps.push(MacroStep::Delay { ms: 50 });
                }
                m.steps.extend(steps);
            }
            st.step = None;
        });
    }

    fn new_macro(&self, bind_selected: bool) {
        self.edit(|_, st| {
            let name = st.cfg.unique_macro_name("New macro");
            st.cfg.macros.push(Macro::new(name.clone(), vec![]));
            if bind_selected && !st.input.is_empty() {
                st.set_action(Action::Macro {
                    name,
                    mode: MacroMode::Once,
                    count: 1,
                    gap_ms: 0,
                });
            }
            st.macro_idx = Some(st.cfg.macros.len() - 1);
            st.step = None;
            st.view = 1;
        });
    }

    /// Fill the selected "Move to" step with the pointer's position, 3 s
    /// from now (time to move the pointer where it should go).
    fn pick_position(&self) {
        if self.st.borrow().picking {
            return;
        }
        self.nav(|_, st| st.picking = true);
        slint::Timer::single_shot(Duration::from_secs(3), || {
            with_app(|app| {
                let pos = system::cursor_pos();
                app.edit(|_, st| {
                    st.picking = false;
                    if let (Some((px, py)), Some(MacroStep::MouseMove { x, y, absolute: true })) =
                        (pos, st.current_step())
                    {
                        *x = px;
                        *y = py;
                    }
                });
            })
        });
    }

    // ---- import / export ---------------------------------------------------------

    fn export(&self, all: bool) {
        let (text, name) = {
            let st = self.st.borrow();
            let indices: Vec<usize> = if all {
                (0..st.cfg.profiles.len()).collect()
            } else {
                vec![st.profile]
            };
            let export = st.cfg.export(&indices);
            let name = if all {
                "mouse-kb-remap-profiles.json".to_string()
            } else {
                let safe: String = st.cfg.profiles[st.profile]
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == ' ' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("{}.json", safe.trim())
            };
            (serde_json::to_string_pretty(&export), name)
        };
        let Ok(text) = text else { return };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export profiles")
            .set_file_name(name)
            .add_filter("Profiles", &["json"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, text) {
            Ok(()) => self.notice(format!("Exported to {}", path.display()), false),
            Err(e) => self.notice(format!("Couldn't write {}: {e}", path.display()), true),
        }
    }

    fn import(&self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import profiles")
            .add_filter("Profiles", &["json"])
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return self.notice(format!("Couldn't read {}: {e}", path.display()), true),
        };
        let mut result = Ok(0);
        self.edit(|_, st| result = st.cfg.import(&text));
        match result {
            Ok(n) => self.notice(
                format!(
                    "Imported {n} profile{} from {}",
                    if n == 1 { "" } else { "s" },
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                false,
            ),
            Err(e) => self.notice(format!("Couldn't import: {e}"), true),
        }
    }

    // ---- updates ---------------------------------------------------------------

    fn auto_check_update(&self) {
        let due = {
            let st = self.st.borrow();
            st.cfg.check_updates && st.update_checked.is_none_or(|t| t.elapsed() >= UPDATE_EVERY)
        };
        if due {
            self.check_update(false);
        }
    }

    /// Look for a new version in the background. `manual`: the user asked,
    /// so say what happened either way.
    fn check_update(&self, manual: bool) {
        if self.st.borrow().update_busy {
            return;
        }
        if manual {
            self.nav(|_, st| {
                st.update_busy = true;
                st.update_status = "Checking…".into();
            });
        }
        std::thread::spawn(move || {
            let result = update::check();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.nav(|_, st| {
                        st.update_busy = false;
                        st.update_checked = Some(Instant::now());
                        match result {
                            Ok(Some(r)) => {
                                st.update_status = format!(
                                    "Version {} is available (you have {}).",
                                    r.version,
                                    update::current_version()
                                );
                                st.update = Some(r);
                            }
                            Ok(None) => {
                                st.update = None;
                                if manual {
                                    st.update_status =
                                        format!("You have the latest version ({}).", update::current_version());
                                }
                            }
                            Err(e) => {
                                crate::log!("update: check failed: {e}");
                                if manual {
                                    st.update_status = format!("Couldn't check for updates: {e}");
                                }
                            }
                        }
                    })
                })
            });
        });
    }

    fn install_update(&self) {
        let (release, hidden) = {
            let st = self.st.borrow();
            match (&st.update, st.update_busy) {
                (Some(r), false) => (r.clone(), !st.window_visible),
                _ => return,
            }
        };
        self.nav(|_, st| {
            st.update_busy = true;
            st.update_status = format!("Downloading version {}…", release.version);
        });
        std::thread::spawn(move || {
            let shown = std::sync::atomic::AtomicU64::new(0);
            let version = release.version.clone();
            let progress = |done: u64, total: u64| {
                if total == 0 {
                    return;
                }
                let percent = done * 100 / total;
                // Every 10 %, not every packet.
                if percent / 10 > shown.load(std::sync::atomic::Ordering::Relaxed) / 10 {
                    shown.store(percent, std::sync::atomic::Ordering::Relaxed);
                    let version = version.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        with_app(|app| {
                            app.nav(|_, st| st.update_status = format!("Downloading version {version}… {percent} %"))
                        })
                    });
                }
            };
            let result = update::install(&release, hidden, &progress);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| match result {
                    Ok(()) => app.quit(),
                    Err(e) => app.nav(|_, st| {
                        st.update_busy = false;
                        st.update_status = format!("Couldn't update: {e}");
                    }),
                })
            });
        });
    }

    /// "Install controller driver": download, check and run the ViGEmBus
    /// installer on a background thread (it waits for the installer).
    fn install_vpad_driver(&self) {
        {
            let mut st = self.st.borrow_mut();
            if st.vpad_installing || !engine::VPAD_SUPPORTED {
                return;
            }
            st.vpad_installing = true;
            st.vpad_message.clear();
        }
        self.nav(|_, _| {});
        std::thread::spawn(|| {
            let result = crate::driver::install(&|_, _| {});
            if let Err(e) = &result {
                crate::log!("driver: not installed: {e}");
            }
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.nav(|_, st| {
                        st.vpad_installing = false;
                        st.vpad_driver = engine::vpad_driver_present();
                        st.vpad_message = match result {
                            Ok(()) => "Driver installed.".into(),
                            Err(e) => format!("Couldn't install it: {e}."),
                        };
                    })
                })
            });
        });
    }

    /// Read the startup entry in the background (schtasks is slow) and note
    /// whether it opens this copy.
    fn check_autostart(&self) {
        std::thread::spawn(|| {
            let status = system::autostart_status();
            let target = if status == Autostart::Off {
                None
            } else {
                system::autostart_target()
            };
            let other = target.filter(|t| !system::is_this_exe(t)).unwrap_or_default();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.nav(|_, st| {
                        st.autostart = status;
                        st.autostart_other = other;
                    })
                })
            });
        });
    }

    /// `force`: set it up again even if the mode is unchanged (to point it at
    /// this copy).
    fn set_autostart(&self, mode: Autostart, force: bool) {
        if mode == self.st.borrow().autostart && !force {
            return;
        }
        self.notice(
            if mode == Autostart::Elevated && !system::is_elevated() {
                "Waiting for the admin prompt…"
            } else {
                "Updating…"
            },
            false,
        );
        // schtasks and the UAC prompt can take a while: don't freeze the window.
        std::thread::spawn(move || {
            let result = system::set_autostart(mode);
            let status = system::autostart_status();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app| {
                    app.st.borrow_mut().autostart = status;
                    app.check_autostart();
                    match result {
                        Ok(()) => app.notice(
                            match status {
                                Autostart::Off => "mouse-kb-remap won't start with Windows.",
                                Autostart::On => "mouse-kb-remap will start with Windows, in the tray.",
                                Autostart::Elevated => {
                                    "mouse-kb-remap will start with Windows as administrator, in the tray."
                                }
                            },
                            false,
                        ),
                        Err(e) => app.notice(format!("Couldn't change startup: {e}"), true),
                    }
                })
            });
        });
    }

    // ---- refresh -------------------------------------------------------------------

    /// Push the whole state into the UI. Cheap enough (a few hundred small
    /// structs) that we don't bother with fine-grained updates.
    fn refresh(&self) {
        let Some(ui) = self.ui() else { return };
        let st = self.st.borrow();
        let s = ui.global::<Store>();
        let cfg = &st.cfg;
        PAD_STYLE.with(|s| s.set(cfg.pad.labels));
        let profile = &cfg.profiles[st.profile];
        let bindings = st.bindings();

        s.set_view(st.view);

        // Sidebar
        s.set_profiles(model(
            cfg.profiles
                .iter()
                .enumerate()
                .map(|(i, p)| ProfileItem {
                    name: p.name.clone().into(),
                    apps: if i == 0 {
                        "Fallback for every app".into()
                    } else if p.apps.is_empty() {
                        "No apps assigned".into()
                    } else {
                        p.apps.join(", ").into()
                    },
                    live: st.live == Some(i),
                    count: p.binding_count() as i32,
                })
                .collect(),
        ));
        s.set_selected_profile(st.profile as i32);
        s.set_macros(model(
            cfg.macros
                .iter()
                .map(|m| {
                    let users = cfg.macro_users(&m.name).len();
                    MacroItem {
                        name: m.name.clone().into(),
                        summary: format!(
                            "{} step{} · {}",
                            m.steps.len(),
                            if m.steps.len() == 1 { "" } else { "s" },
                            match users {
                                0 => "not bound".to_string(),
                                1 => "used by 1 profile".to_string(),
                                n => format!("used by {n} profiles"),
                            }
                        )
                        .into(),
                    }
                })
                .collect(),
        ));
        s.set_macro_names(model(cfg.macros.iter().map(|m| SharedString::from(&m.name)).collect()));
        s.set_selected_macro(st.macro_idx.map(|i| i as i32).unwrap_or(-1));
        s.set_update_available(st.update.as_ref().map(|r| r.version.clone()).unwrap_or_default().into());

        // Header
        s.set_profile_name(profile.name.clone().into());
        s.set_profile_apps(model(profile.apps.iter().map(SharedString::from).collect()));
        s.set_profile_fullscreen(profile.fullscreen_only);
        s.set_recent_apps(model(
            st.recent
                .iter()
                .filter(|r| !profile.apps.iter().any(|a| a.eq_ignore_ascii_case(r)))
                .take(4)
                .map(SharedString::from)
                .collect(),
        ));
        s.set_layer(st.layer as i32);
        s.set_layer_banner(layer_banner(cfg, st.profile, st.layer).into());
        s.set_full_keyboard(cfg.full_keyboard);

        // Device shown, and what can't be bound for it
        let suffix = st.device_suffix();
        let mut device_names = vec![SharedString::from("All devices")];
        device_names.extend(cfg.devices.iter().map(|d| SharedString::from(&d.name)));
        s.set_device_names(model(device_names));
        s.set_device_current(if st.shown_device().is_some() {
            st.device as i32
        } else {
            0
        });
        let tab = s.get_device_tab();
        s.set_device_note(
            match st.shown_device() {
                Some(d) if d.is_controller() && tab != 2 => {
                    format!("{} is a controller: its bindings are on the Controller tab.", d.name)
                }
                Some(d) if !d.is_controller() && d.kind() != "Input device" && tab == 2 => {
                    format!("{} is a mouse: its bindings are on the Mouse tab.", d.name)
                }
                Some(d) if tab == 1 && !KEYBOARDS_PER_DEVICE => format!(
                    "Windows can't tell which keyboard a key came from, so keys can't have bindings just for {}. \
                     Keyboard bindings are under \"All devices\".",
                    d.name
                ),
                _ => String::new(),
            }
            .into(),
        );

        // Controller
        s.set_pad_buttons(model(
            PadButton::ALL
                .iter()
                .map(|b| {
                    let id = format!("pad:{}{suffix}", b.id());
                    let action = bindings.get(&id);
                    let value = match action {
                        Some(a) => a.label(),
                        None => inherited_short(cfg, st.profile, st.layer, &id).unwrap_or_else(|| "—".into()),
                    };
                    MouseBtn {
                        id: id.clone().into(),
                        label: short_pad_label(*b, cfg.pad.labels).into(),
                        detail: b.detail().into(),
                        value: value.into(),
                        mapped: action.is_some(),
                        supported: true,
                    }
                })
                .collect(),
        ));
        s.set_pad_glyphs(model(
            PadButton::ALL
                .iter()
                .map(|b| SharedString::from(pad_glyph(*b, cfg.pad.labels)))
                .collect(),
        ));
        let (stick_items, stick_current) = if st.profile == 0 {
            (
                vec!["Off", "Left", "Right"],
                match profile.stick_pointer.unwrap_or_default() {
                    StickPointer::Off => 0,
                    StickPointer::Left => 1,
                    StickPointer::Right => 2,
                },
            )
        } else {
            (
                vec!["As Default", "Off", "Left", "Right"],
                match profile.stick_pointer {
                    None => 0,
                    Some(StickPointer::Off) => 1,
                    Some(StickPointer::Left) => 2,
                    Some(StickPointer::Right) => 3,
                },
            )
        };
        s.set_stick_items(model(stick_items.into_iter().map(SharedString::from).collect()));
        s.set_stick_current(stick_current);
        s.set_stick_speed(cfg.stick_speed.to_string().into());
        // Gyro / touchpad: Off / On, or (other profiles) Same as Default first.
        let on_off = |own: Option<bool>| -> (Vec<SharedString>, i32) {
            if st.profile == 0 {
                (vec!["Off".into(), "On".into()], own.unwrap_or(false) as i32)
            } else {
                (
                    vec!["As Default".into(), "Off".into(), "On".into()],
                    match own {
                        None => 0,
                        Some(false) => 1,
                        Some(true) => 2,
                    },
                )
            }
        };
        let (items, current) = on_off(profile.gyro_pointer);
        s.set_gyro_items(model(items));
        s.set_gyro_current(current);
        let (items, current) = on_off(profile.touch_pointer);
        s.set_touch_items(model(items));
        s.set_touch_current(current);
        s.set_pad_detecting(st.detecting);
        if let Some(Action::Gyro { toggle }) = st.action() {
            s.set_gyro_toggle(*toggle);
        }
        let p = &cfg.pad;
        s.set_pad_gyro_speed(p.gyro_speed.to_string().into());
        s.set_pad_touch_speed(p.touch_speed.to_string().into());
        s.set_pad_invert_x(p.gyro_invert_x);
        s.set_pad_invert_y(p.gyro_invert_y);
        s.set_pad_labels(match p.labels {
            PadLabels::Xbox => 0,
            PadLabels::PlayStation => 1,
            PadLabels::Nintendo => 2,
        });
        s.set_pad_xinput(p.xinput);
        s.set_pad_direct(p.direct);
        s.set_pad_skip_steam(p.skip_steam_virtual);
        s.set_pad_lizard(p.steam_lizard_off);
        s.set_pad_copy(p.copy_to_virtual);
        s.set_vpad_supported(engine::VPAD_SUPPORTED);
        let (state, text) = vpad_display(&st);
        s.set_vpad_state(state);
        s.set_vpad_text(text.into());
        s.set_pad_output_names(model(
            PadButton::OUTPUTS
                .iter()
                .map(|b| SharedString::from(output_grid_name(*b)))
                .collect(),
        ));
        s.set_pad_output_current(match st.action() {
            Some(Action::Pad { button }) => output_index(*button),
            _ => -1,
        });

        // Keyboard
        s.set_keys(model(
            keys::layout(cfg.full_keyboard)
                .map(|k| {
                    let id = format!("key:{}{suffix}", k.id);
                    let action = bindings.get(&id);
                    KeyCap {
                        id: id.into(),
                        label: k.label.into(),
                        x: k.x,
                        y: k.y,
                        w: k.w,
                        h: k.h,
                        mapped: action.is_some(),
                        value: action.map(|a| a.short_label()).unwrap_or_default().into(),
                    }
                })
                .collect(),
        ));

        // Mouse
        s.set_mouse_buttons(model(
            MOUSE_ORDER
                .iter()
                .map(|m| {
                    let id = format!("mouse:{m}{suffix}");
                    if *m == "dpi" {
                        return MouseBtn {
                            id: id.into(),
                            label: "DPI button".into(),
                            detail: "Firmware".into(),
                            value: "Firmware".into(),
                            mapped: false,
                            supported: false,
                        };
                    }
                    let action = bindings.get(&id);
                    let value = match action {
                        Some(a) => a.label(),
                        None => inherited_short(cfg, st.profile, st.layer, &id).unwrap_or_else(|| "Default".into()),
                    };
                    MouseBtn {
                        id: id.clone().into(),
                        label: plain_label(split_device(&id).0).into(),
                        detail: input_detail(split_device(&id).0).into(),
                        value: value.into(),
                        mapped: action.is_some(),
                        supported: true,
                    }
                })
                .collect(),
        ));

        // All bindings of this profile (by layer, mouse before keys, combos last)
        let mut rows: Vec<BindingRow> = (0..LAYER_COUNT)
            .flat_map(|layer| {
                profile.layer(layer).iter().map(move |(input, action)| BindingRow {
                    input_id: input.clone().into(),
                    input: input_label(input).into(),
                    action: action.label().into(),
                    layer: layer as i32,
                })
            })
            .collect();
        rows.sort_by_key(|r| {
            let combo = r.input_id.contains('+');
            (r.layer, combo, !r.input_id.starts_with("mouse:"))
        });
        s.set_bindings(model(rows));

        // Inspector
        let input = st.input.as_str();
        let combo = Combo::parse(input);
        s.set_selected_input(input.into());
        s.set_selected_label(input_label(input).into());
        let is_dpi = split_device(input).0 == "mouse:dpi";
        s.set_selected_detail(
            if is_dpi {
                "Handled by firmware".into()
            } else {
                input_detail(input)
            }
            .into(),
        );
        s.set_selected_supported(!is_dpi);
        s.set_selected_is_combo(combo.is_some());
        if let Some(c) = &combo {
            s.set_combo_mods(Modifier::to_bits(&c.mods) as i32);
            let held_mouse = c
                .held
                .iter()
                .find_map(|h| h.strip_prefix("mouse:").and_then(MouseButton::from_id))
                .and_then(|b| COMBO_MOUSE.iter().position(|m| *m == b))
                .map(|i| i as i32 + 1)
                .unwrap_or(0);
            s.set_combo_held_mouse(held_mouse);
            s.set_combo_held_key(
                c.held
                    .iter()
                    .find(|h| h.starts_with("key:"))
                    .map(|h| plain_label(h))
                    .unwrap_or_default()
                    .into(),
            );
            s.set_combo_trigger(plain_label(&c.trigger).into());
            s.set_combos(model(vec![]));
        } else {
            s.set_combos(model(
                bindings
                    .iter()
                    .filter(|(id, _)| Combo::parse(id).is_some_and(|c| c.trigger == input))
                    .map(|(id, a)| BindingRow {
                        input_id: id.clone().into(),
                        input: input_label(id).into(),
                        action: a.label().into(),
                        layer: st.layer as i32,
                    })
                    .collect(),
            ));
        }

        let action = st.action();
        let kind = st
            .pending_kind
            .unwrap_or_else(|| action.map(kind_of).unwrap_or(K_DEFAULT));
        s.set_selected_kind(kind);
        s.set_slot(st.slot as i32);
        let gesture = matches!(st.binding(), Some(Action::Gesture { .. }));
        s.set_slot_title(
            if gesture {
                GESTURE_TITLES[st.slot.min(5)]
            } else {
                SLOT_TITLES[st.slot.min(3)]
            }
            .into(),
        );
        s.set_slot_back(if gesture { "← Gesture" } else { "← Tap / hold" }.into());
        if let Some(Action::Gesture { distance, .. }) = st.binding() {
            let parts: Vec<SharedString> = (1..=5)
                .map(|n| {
                    st.binding()
                        .and_then(|a| a.slot(n))
                        .map(|a| a.label())
                        .unwrap_or_else(|| "Nothing".into())
                        .into()
                })
                .collect();
            s.set_gesture_parts(model(parts));
            s.set_gesture_distance(distance.to_string().into());
        }
        if st.slot == 0 {
            s.set_kind_items(model(KIND_NAMES.iter().map(|n| SharedString::from(*n)).collect()));
            s.set_kind_current(kind);
            s.set_inherited_value(inherited(cfg, st.profile, st.layer, input).unwrap_or_default().into());
        } else {
            s.set_kind_items(model(SLOT_NAMES.iter().map(|n| SharedString::from(*n)).collect()));
            s.set_kind_current(
                SLOT_KINDS
                    .iter()
                    .position(|k| *k == kind)
                    .map(|i| i as i32)
                    .unwrap_or(-1),
            );
            s.set_inherited_value(
                match (gesture, st.slot) {
                    (true, 1) => "Nothing happens when you let go without moving.",
                    (true, _) => "Nothing happens for this direction.",
                    (false, 1) => "Nothing happens on a tap.",
                    (false, 2) => "Nothing happens on a hold.",
                    _ => "No double tap: a second tap is just another tap.",
                }
                .into(),
            );
        }
        if let Some(Action::TapHold { hold_ms, double_ms, .. }) = st.binding() {
            let part = |n: usize| {
                st.binding()
                    .and_then(|a| a.slot(n))
                    .map(|a| a.label())
                    .unwrap_or_else(|| "Nothing".into())
            };
            s.set_taphold_tap(part(1).into());
            s.set_taphold_hold(part(2).into());
            s.set_taphold_double(part(3).into());
            s.set_taphold_hold_ms(hold_ms.to_string().into());
            s.set_taphold_double_ms(double_ms.to_string().into());
        }

        let (key_value, key_mods) = match action {
            Some(Action::Key { key, mods }) if !keys::MEDIA.contains(&key.as_str()) => {
                (combo_label(key, mods), Modifier::to_bits(mods))
            }
            _ => (String::new(), 0),
        };
        s.set_key_value(key_value.into());
        s.set_key_mods(key_mods as i32);
        s.set_shortcut_current(
            match action {
                Some(Action::Key { key, mods }) => keys::SHORTCUTS
                    .iter()
                    .position(|(_, k, m)| k == key && Modifier::to_bits(m) == Modifier::to_bits(mods))
                    .map(|i| i as i32),
                _ => None,
            }
            .unwrap_or(-1),
        );
        s.set_mouse_current(match action {
            Some(Action::Mouse { button }) => MouseButton::ALL.iter().position(|b| b == button).unwrap_or(0) as i32,
            _ => -1,
        });
        s.set_media_current(match action {
            Some(Action::Key { key, .. }) => keys::MEDIA
                .iter()
                .position(|m| m == key)
                .map(|i| i as i32)
                .unwrap_or(-1),
            _ => -1,
        });
        s.set_system_current(match action {
            Some(Action::System { command }) => SystemCommand::ALL
                .iter()
                .position(|c| c == command)
                .map(|i| i as i32)
                .unwrap_or(-1),
            _ => -1,
        });
        if let Some(Action::Layer { layer, toggle }) = action {
            s.set_layer_target(*layer as i32);
            s.set_layer_toggle(*toggle);
        }

        if let Some(Action::Macro {
            name,
            mode,
            count,
            gap_ms,
        }) = action
        {
            s.set_macro_current(
                cfg.macros
                    .iter()
                    .position(|m| &m.name == name)
                    .map(|i| i as i32)
                    .unwrap_or(-1),
            );
            s.set_macro_mode(mode.index());
            s.set_macro_count(count.to_string().into());
            s.set_macro_gap(gap_ms.to_string().into());
        } else {
            s.set_macro_current(-1);
            s.set_macro_mode(0);
        }

        if let Some(Action::Turbo {
            output,
            interval_ms,
            hold_ms,
            delay_ms,
            toggle,
        }) = action
        {
            match output {
                Output::Key { key, mods } => {
                    s.set_turbo_kind(0);
                    s.set_turbo_key(combo_label(key, mods).into());
                    s.set_turbo_mods(Modifier::to_bits(mods) as i32);
                    s.set_turbo_mouse(-1);
                }
                Output::Mouse { button } => {
                    s.set_turbo_kind(1);
                    s.set_turbo_key("".into());
                    s.set_turbo_mods(0);
                    s.set_turbo_mouse(MouseButton::ALL.iter().position(|b| b == button).unwrap_or(0) as i32);
                }
                Output::Pad { button } => {
                    s.set_turbo_kind(2);
                    s.set_turbo_key("".into());
                    s.set_turbo_mods(0);
                    s.set_turbo_mouse(-1);
                    s.set_turbo_pad(output_index(*button));
                }
            }
            s.set_turbo_interval(interval_ms.to_string().into());
            s.set_turbo_hold(hold_ms.to_string().into());
            s.set_turbo_delay(delay_ms.to_string().into());
            s.set_turbo_toggle(*toggle);
            s.set_turbo_rate(format!("{}/s", per_second(*interval_ms)).into());
        }

        if let Some(Action::Text { text, delay_ms }) = action {
            s.set_text_value(text.clone().into());
            s.set_text_delay(delay_ms.to_string().into());
        } else {
            s.set_text_value("".into());
        }
        if let Some(Action::Launch { path, args }) = action {
            s.set_launch_path(path.clone().into());
            s.set_launch_args(args.clone().into());
        } else {
            s.set_launch_path("".into());
            s.set_launch_args("".into());
        }
        s.set_sens_speed(match action {
            Some(Action::Sensitivity { speed }) => *speed as i32,
            _ => 10,
        });

        s.set_capturing(st.capturing.clone().into());

        // Key picker ("Choose from a list")
        s.set_picker_target(st.picker_target.clone().into());
        if !st.picker_target.is_empty() {
            let group = st.picker_group.min(PICKER_GROUPS.len() - 1);
            let list = picker_keys(group);
            let current = picker_current(&st, &st.picker_target);
            s.set_picker_groups(model(PICKER_GROUPS.iter().map(|g| SharedString::from(*g)).collect()));
            s.set_picker_group(group as i32);
            s.set_picker_keys(model(
                list.iter()
                    .map(|k| SharedString::from(keys::display_name(k.id)))
                    .collect(),
            ));
            s.set_picker_current(
                current
                    .and_then(|c| list.iter().position(|k| k.id == c))
                    .map_or(-1, |i| i as i32),
            );
            s.set_picker_columns(match group {
                0..=2 => 4,
                8 => 2,
                _ => 3,
            });
        }

        // Macro editor
        let m = st.macro_idx.and_then(|i| cfg.macros.get(i));
        s.set_macro_name(m.map(|m| m.name.clone()).unwrap_or_default().into());
        s.set_macro_jitter(m.map(|m| m.jitter_ms.to_string()).unwrap_or_default().into());
        s.set_steps(model(
            m.map(|m| {
                m.steps
                    .iter()
                    .map(|step| {
                        let (kind, value) = step.describe();
                        StepRow {
                            kind: kind.into(),
                            value: value.into(),
                            delay: matches!(step, MacroStep::Delay { .. }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ));
        s.set_recording(st.recording);
        s.set_picking_position(st.picking);
        if let Some(m) = m {
            let total: u64 = m
                .steps
                .iter()
                .map(|s| match s {
                    MacroStep::Delay { ms } => *ms as u64,
                    MacroStep::KeyTap { hold_ms, .. } | MacroStep::MouseClick { hold_ms, .. } => *hold_ms as u64,
                    _ => 0,
                })
                .sum();
            let users = cfg.macro_users(&m.name);
            s.set_macro_info(
                format!(
                    "{} step{} · plays in about {:.2} s · {}",
                    m.steps.len(),
                    if m.steps.len() == 1 { "" } else { "s" },
                    total as f64 / 1000.0,
                    if users.is_empty() {
                        "not bound to any input yet".to_string()
                    } else {
                        format!("used by {}", users.join(", "))
                    }
                )
                .into(),
            );
        }
        let step = m.and_then(|m| st.step.and_then(|i| m.steps.get(i)));
        s.set_selected_step(if step.is_some() { st.step.unwrap() as i32 } else { -1 });
        if let Some(step) = step {
            s.set_step_kind(step_kind(step));
            s.set_step_key(step_key(step).map(keys::display_name).unwrap_or_default().into());
            s.set_step_mouse(
                step_button(step)
                    .and_then(|b| MouseButton::ALL.iter().position(|x| *x == b))
                    .map(|i| i as i32)
                    .unwrap_or(-1),
            );
            s.set_step_ms(step_ms(step).map(|v| v.to_string()).unwrap_or_default().into());
            s.set_step_text(
                match step {
                    MacroStep::Text { text } => text.clone(),
                    _ => String::new(),
                }
                .into(),
            );
            let (x, y) = step_xy(step).unwrap_or((0, 0));
            s.set_step_x(x.to_string().into());
            s.set_step_y(y.to_string().into());
        } else {
            s.set_step_kind(-1);
        }

        // Settings
        s.set_autostart(st.autostart.index());
        s.set_autostart_other(st.autostart_other.clone().into());
        s.set_close_to_tray(cfg.close_to_tray);
        s.set_notice(st.notice.0.clone().into());
        s.set_notice_error(st.notice.1);
        s.set_hotkey_values(model(
            (0..Hotkeys::NAMES.len())
                .map(|i| {
                    cfg.hotkeys
                        .get(i)
                        .map(|h| SharedString::from(combo_label(&h.key, &h.mods)))
                        .unwrap_or_default()
                })
                .collect(),
        ));
        s.set_notifications(cfg.notifications);
        s.set_check_updates(cfg.check_updates);
        s.set_devices(model(
            cfg.devices
                .iter()
                .map(|d| DeviceItem {
                    name: d.name.clone().into(),
                    detail: format!("{} · {}", d.kind(), device_id_summary(&d.id)).into(),
                })
                .collect(),
        ));
        s.set_adding_device(st.adding_device);
        s.set_keyboards_per_device(KEYBOARDS_PER_DEVICE);
        s.set_update_status(st.update_status.clone().into());
        s.set_update_busy(st.update_busy);

        // Status bar
        s.set_focused_app(st.focus_exe.clone().into());
        let live_name = cfg.profiles[st.shown_live()].name.clone();
        s.set_live_profile(
            if st.focus_self && !st.focus_exe.is_empty() {
                format!("{live_name} (paused while this window is focused)")
            } else {
                live_name.clone()
            }
            .into(),
        );
        s.set_live_forced(st.forced);
        s.set_elevation_warning(st.elevation_blocked && !st.focus_self);
        s.set_engine_enabled(cfg.enabled);
        s.set_can_undo(!st.undo.is_empty());
        s.set_can_redo(!st.redo.is_empty());

        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_remap_enabled(cfg.enabled);
            tray.set_status(
                if cfg.enabled {
                    format!("{live_name} profile")
                } else {
                    "paused".to_string()
                }
                .into(),
            );
        }
    }
}

fn wire_callbacks(ui: &AppWindow, app: &Rc<App>) {
    let s = ui.global::<Store>();

    // Shorthands: `edit` saves + applies, `nav` only redraws.
    macro_rules! on {
        ($setter:ident, |$($arg:ident),*| edit $body:expr) => {{
            let a = app.clone();
            s.$setter(move |$($arg),*| a.edit(|_app, st| { let _ = &st; ($body)(st) }));
        }};
        ($setter:ident, |$($arg:ident),*| nav $body:expr) => {{
            let a = app.clone();
            s.$setter(move |$($arg),*| a.nav(|_app, st| { let _ = &st; ($body)(st) }));
        }};
        ($setter:ident, |$($arg:ident),*| app $body:expr) => {{
            let a = app.clone();
            s.$setter(move |$($arg),*| ($body)(&a));
        }};
    }

    // ---- navigation & profiles ------------------------------------------------------
    {
        let a = app.clone();
        s.on_open_settings(move || {
            a.nav(|_, st| st.view = 2);
            a.check_autostart();
        });
    }
    on!(on_select_profile, |i| nav |st: &mut State| {
        st.view = 0;
        st.profile = (i.max(0) as usize).min(st.cfg.profiles.len() - 1);
        st.pending_kind = None;
        st.slot = 0;
    });
    on!(on_add_profile, | | edit |st: &mut State| {
        let name = st.cfg.unique_profile_name("New profile");
        st.cfg.profiles.push(Profile::new(name));
        st.profile = st.cfg.profiles.len() - 1;
        st.layer = 0;
        st.pending_kind = None;
        st.view = 0;
    });
    on!(on_duplicate_profile, | | edit |st: &mut State| {
        let mut copy = st.cfg.profiles[st.profile].clone();
        // Two profiles for the same app would fight; the copy starts unassigned.
        copy.apps.clear();
        copy.name = st.cfg.unique_profile_name(&format!("{} copy", copy.name));
        st.cfg.profiles.push(copy);
        st.profile = st.cfg.profiles.len() - 1;
        st.pending_kind = None;
    });
    {
        let a = app.clone();
        s.on_delete_profile(move || {
            if a.st.borrow().profile == 0 {
                return;
            }
            // Profile numbers shift: a profile chosen by hand goes back to automatic.
            if a.st.borrow().forced {
                a.engine.force_profile(None);
            }
            a.edit(|_, st| {
                st.cfg.profiles.remove(st.profile);
                st.profile -= 1;
                st.pending_kind = None;
            });
        });
    }
    {
        let a = app.clone();
        s.on_rename_profile(move |name| {
            let mut st = a.st.borrow_mut();
            let name = name.trim();
            if st.profile == 0 || name.is_empty() || st.cfg.profiles[st.profile].name == name {
                return;
            }
            let before = Snapshot::of(&st.cfg);
            let p = st.profile;
            st.cfg.profiles[p].name = name.to_string();
            st.push_undo(before);
            st.cfg.save();
            drop(st);
            a.refresh();
        });
    }
    on!(on_add_app, |app_name| edit |st: &mut State| {
        let mut name = app_name.trim().to_string();
        if name.is_empty() || st.profile == 0 {
            return;
        }
        let plain = !name.starts_with("title:") && !name.contains(['\\', '/', '*', '?', '.']);
        if plain {
            name.push_str(".exe");
        }
        let p = st.profile;
        let apps = &mut st.cfg.profiles[p].apps;
        if !apps.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            apps.push(name);
        }
    });
    on!(on_remove_app, |app_name| edit |st: &mut State| {
        let p = st.profile;
        st.cfg.profiles[p].apps.retain(|a| a.as_str() != app_name.as_str());
    });
    on!(on_set_profile_fullscreen, |on| edit |st: &mut State| {
        let p = st.profile;
        if p > 0 {
            st.cfg.profiles[p].fullscreen_only = on;
        }
    });
    on!(on_select_layer, |layer| nav |st: &mut State| {
        st.layer = layer.clamp(0, LAYER_COUNT as i32 - 1) as usize;
        st.pending_kind = None;
        st.slot = 0;
    });
    on!(on_set_full_keyboard, |on| edit |st: &mut State| st.cfg.full_keyboard = on);

    // ---- bindings ----------------------------------------------------------------------
    {
        let a = app.clone();
        s.on_select_input(move |id| {
            a.cancel_capture();
            a.nav(|_, st| st.select_input(&id));
        });
    }
    {
        let a = app.clone();
        s.on_select_binding(move |id, layer| {
            let shown = Combo::parse(&id).map(|c| c.trigger).unwrap_or_else(|| id.to_string());
            let (shown, device) = split_device(&shown);
            if let Some(ui) = a.ui() {
                ui.global::<Store>().set_inspector_tab(0);
                if let Some(key) = shown.strip_prefix("key:") {
                    let on_drawing = keys::find(key).is_some_and(|k| k.group != keys::Group::Extra);
                    if on_drawing {
                        ui.global::<Store>().set_device_tab(1);
                    }
                } else if shown.starts_with("pad:") {
                    ui.global::<Store>().set_device_tab(2);
                } else {
                    ui.global::<Store>().set_device_tab(0);
                }
            }
            let device = device.map(str::to_string);
            a.nav(|_, st| {
                st.select_input(&id);
                st.layer = layer.clamp(0, LAYER_COUNT as i32 - 1) as usize;
                st.device = device
                    .and_then(|d| st.cfg.devices.iter().position(|x| x.name == d))
                    .map_or(0, |i| i + 1);
            });
        });
    }
    on!(on_clear_binding, |id, layer| edit |st: &mut State| {
        let p = st.profile;
        st.cfg.profiles[p].layer_mut(layer.clamp(0, LAYER_COUNT as i32 - 1) as usize).remove(id.as_str());
        st.pending_kind = None;
    });
    {
        let a = app.clone();
        s.on_set_kind(move |i| {
            a.cancel_capture();
            a.edit(|_, st| {
                let kind = if st.slot == 0 {
                    i
                } else {
                    SLOT_KINDS.get(i.max(0) as usize).copied().unwrap_or(K_DEFAULT)
                };
                set_kind(st, kind)
            });
        });
    }
    {
        let a = app.clone();
        s.on_edit_slot(move |slot| {
            a.cancel_capture();
            a.nav(|_, st| {
                st.slot = slot.clamp(0, 5) as usize;
                st.pending_kind = None;
            });
        });
    }
    on!(on_set_gesture_distance, |value| edit |st: &mut State| {
        let Some(v) = parse_ms(&value) else { return };
        if let Some(Action::Gesture { distance, .. }) = st.binding_mut() {
            *distance = v.clamp(5, 2000);
        }
    });
    on!(on_select_device, |i| nav |st: &mut State| {
        st.device = (i.max(0) as usize).min(st.cfg.devices.len());
        // The same input, on the device picked.
        let (base, _) = split_device(&st.input);
        let base = base.to_string();
        let id = if base.is_empty() { base } else { format!("{base}{}", st.device_suffix()) };
        st.select_input(&id);
    });
    on!(on_tab_changed, | | nav |_st: &mut State| {});
    on!(on_set_stick, |i| edit |st: &mut State| {
        let p = st.profile;
        let choice = if p == 0 { i + 1 } else { i };
        st.cfg.profiles[p].stick_pointer = match choice {
            1 => Some(StickPointer::Off),
            2 => Some(StickPointer::Left),
            3 => Some(StickPointer::Right),
            _ => None,
        };
        if p == 0 && st.cfg.profiles[0].stick_pointer == Some(StickPointer::Off) {
            st.cfg.profiles[0].stick_pointer = None;
        }
    });
    on!(on_set_stick_speed, |value| edit |st: &mut State| {
        if let Ok(v) = value.trim().parse::<u32>() {
            st.cfg.stick_speed = v.clamp(1, 20);
        }
    });
    on!(on_add_device, | | app |a: &Rc<App>| a.toggle_add_device());
    {
        let a = app.clone();
        s.on_toggle_picker(move |target| {
            a.cancel_capture();
            a.nav(|_, st| {
                if st.picker_target == target.as_str() {
                    st.picker_target.clear();
                } else {
                    // Open on the group of the key chosen now.
                    st.picker_group = picker_current(st, &target)
                        .and_then(|k| keys::find(&k).map(picker_group_of))
                        .unwrap_or(0);
                    st.picker_target = target.to_string();
                }
            });
        });
    }
    on!(on_pick_group, |i| nav |st: &mut State| st.picker_group = (i.max(0) as usize).min(PICKER_GROUPS.len() - 1));
    {
        let a = app.clone();
        s.on_pick_key(move |target, i| {
            let (key, mods) = {
                let st = a.st.borrow();
                let Some(k) = picker_keys(st.picker_group).get(i.max(0) as usize).map(|k| k.id) else {
                    return;
                };
                // Keep the modifiers already chosen for this key.
                let mods = match (target.as_str(), st.action()) {
                    ("binding", Some(Action::Key { mods, .. })) => Modifier::to_bits(mods),
                    (
                        "turbo",
                        Some(Action::Turbo {
                            output: Output::Key { mods, .. },
                            ..
                        }),
                    ) => Modifier::to_bits(mods),
                    _ => 0,
                };
                (k, mods)
            };
            a.cancel_capture();
            a.st.borrow_mut().capturing = target.to_string();
            a.captured(key, mods);
        });
    }
    {
        let a = app.clone();
        s.on_detect_pad(move || {
            let on = !a.st.borrow().detecting;
            a.engine.detect_pad(on);
            a.nav(|_, st| st.detecting = on);
        });
    }
    // Gyro / touchpad pointer: (Same as Default,) Off, On.
    fn on_off_choice(profile: usize, i: i32) -> Option<bool> {
        match if profile == 0 { i + 1 } else { i } {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }
    on!(on_set_gyro, |i| edit |st: &mut State| {
        let p = st.profile;
        st.cfg.profiles[p].gyro_pointer = on_off_choice(p, i).filter(|on| p > 0 || *on);
    });
    on!(on_set_touch, |i| edit |st: &mut State| {
        let p = st.profile;
        st.cfg.profiles[p].touch_pointer = on_off_choice(p, i).filter(|on| p > 0 || *on);
    });
    on!(on_set_gyro_toggle, |on| edit |st: &mut State| {
        if let Some(Action::Gyro { toggle }) = st.action_mut() {
            *toggle = on;
        }
    });
    on!(on_set_pad_number, |field, value| edit |st: &mut State| {
        let Ok(v) = value.trim().parse::<u32>() else { return };
        let v = v.clamp(1, 20);
        match field.as_str() {
            "gyro" => st.cfg.pad.gyro_speed = v,
            _ => st.cfg.pad.touch_speed = v,
        }
    });
    on!(on_set_pad_flag, |field, on| edit |st: &mut State| {
        let p = &mut st.cfg.pad;
        match field.as_str() {
            "invert_x" => p.gyro_invert_x = on,
            "invert_y" => p.gyro_invert_y = on,
            "xinput" => p.xinput = on,
            "direct" => p.direct = on,
            "skip_steam" => p.skip_steam_virtual = on,
            "copy" => p.copy_to_virtual = on,
            _ => p.steam_lizard_off = on,
        }
    });
    on!(on_set_pad_labels, |i| edit |st: &mut State| {
        st.cfg.pad.labels = match i {
            1 => PadLabels::PlayStation,
            2 => PadLabels::Nintendo,
            _ => PadLabels::Xbox,
        };
    });
    {
        let a = app.clone();
        s.on_rename_device(move |i, name| {
            let name = clean_device_name(&name);
            a.edit(|_, st| {
                let Some(old) = st.cfg.devices.get(i.max(0) as usize).map(|d| d.name.clone()) else {
                    return;
                };
                if name.is_empty() || name == old {
                    return;
                }
                if st
                    .cfg
                    .devices
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case(&name) && d.name != old)
                {
                    st.notice = (format!("There is already a device called \"{name}\"."), true);
                    return;
                }
                st.cfg.retarget_device(&old, Some(&name));
                let (base, dev) = split_device(&st.input);
                if dev == Some(old.as_str()) {
                    st.input = format!("{base}@{name}");
                }
                st.notice = (String::new(), false);
            });
        });
    }
    {
        let a = app.clone();
        s.on_remove_device(move |i| {
            a.edit(|_, st| {
                let Some(name) = st.cfg.devices.get(i.max(0) as usize).map(|d| d.name.clone()) else {
                    return;
                };
                let n = st.cfg.retarget_device(&name, None);
                st.device = 0;
                let base = split_device(&st.input).0.to_string();
                st.select_input(&base);
                st.notice = (
                    match n {
                        0 => format!("Removed \"{name}\"."),
                        1 => format!("Removed \"{name}\" and its binding."),
                        n => format!("Removed \"{name}\" and its {n} bindings."),
                    },
                    false,
                );
            });
        });
    }
    on!(on_set_taphold_number, |field, value| edit |st: &mut State| {
        let Some(v) = parse_ms(&value) else { return };
        if let Some(Action::TapHold { hold_ms, double_ms, .. }) = st.binding_mut() {
            match field.as_str() {
                "hold" => *hold_ms = v.clamp(10, 5000),
                _ => *double_ms = v.clamp(10, 5000),
            }
        }
    });
    on!(on_set_layer_target, |layer| edit |st: &mut State| {
        if let Some(Action::Layer { layer: l, .. }) = st.action_mut() {
            *l = layer.clamp(1, LAYER_COUNT as i32 - 1) as u8;
        }
    });
    on!(on_set_layer_toggle, |on| edit |st: &mut State| {
        if let Some(Action::Layer { toggle, .. }) = st.action_mut() {
            *toggle = on;
        }
    });
    on!(on_set_system, |i| edit |st: &mut State| {
        if let Some(command) = SystemCommand::ALL.get(i.max(0) as usize) {
            st.set_action(Action::System { command: *command });
        }
    });
    on!(on_add_combo, | | nav |st: &mut State| {
        let trigger = st.input.clone();
        if trigger.is_empty() || Combo::parse(&trigger).is_some() {
            return;
        }
        // The first modifier combo with this input that isn't taken yet.
        for m in Modifier::ALL {
            let id = Combo { mods: vec![m], held: vec![], trigger: trigger.clone() }.id();
            if !st.bindings().contains_key(&id) {
                st.select_input(&id);
                break;
            }
        }
    });
    on!(on_remove_combo, | | edit |st: &mut State| {
        if let Some(c) = Combo::parse(&st.input) {
            let id = st.input.clone();
            st.bindings_mut().remove(&id);
            st.select_input(&c.trigger);
        }
    });
    on!(on_back_to_trigger, | | nav |st: &mut State| {
        if let Some(c) = Combo::parse(&st.input) {
            st.select_input(&c.trigger);
        }
    });
    on!(on_set_combo_mouse, |i| edit |st: &mut State| {
        let held = (i > 0)
            .then(|| COMBO_MOUSE.get(i as usize - 1))
            .flatten()
            .map(|b| format!("mouse:{}", b.id()));
        st.update_combo(|c| c.held = held.into_iter().collect());
    });
    on!(on_start_capture, |target| app |a: &Rc<App>| a.start_capture(&target));
    on!(on_cancel_capture, | | app |a: &Rc<App>| a.cancel_capture());
    {
        let a = app.clone();
        s.on_ui_key(move |text, released, mods| {
            let Some(key) = keys::from_slint_text(&text) else {
                return;
            };
            let is_mod = keys::find(key).and_then(|k| keys::modifier_of(k.vk)).is_some();
            let lone = {
                let mut st = a.st.borrow_mut();
                match (is_mod, released) {
                    // Wait: a modifier may start a combo.
                    (true, false) => {
                        st.ui_lone_mod = Some(key);
                        return;
                    }
                    (true, true) => st.ui_lone_mod.take() == Some(key),
                    (false, false) => {
                        st.ui_lone_mod = None;
                        false
                    }
                    (false, true) => return,
                }
            };
            if lone {
                a.captured(key, 0);
            } else if !is_mod {
                a.captured(key, mods as u8);
            }
        });
    }
    on!(on_toggle_mod, |target, bit| edit |st: &mut State| {
        let bit = bit as u8;
        let toggle = |mods: &mut Vec<Modifier>| {
            let bits = Modifier::to_bits(mods) ^ bit;
            *mods = Modifier::from_bits(bits);
        };
        if target.as_str() == "combo" {
            st.update_combo(|c| toggle(&mut c.mods));
            return;
        }
        match (target.as_str(), st.action_mut()) {
            ("binding", Some(Action::Key { mods, .. })) => toggle(mods),
            ("turbo", Some(Action::Turbo { output: Output::Key { mods, .. }, .. })) => toggle(mods),
            _ => {}
        }
    });
    on!(on_set_shortcut, |i| edit |st: &mut State| {
        if let Some((_, key, mods)) = keys::SHORTCUTS.get(i as usize) {
            st.set_action(Action::Key { key: key.to_string(), mods: mods.to_vec() });
        }
    });
    on!(on_set_mouse_target, |i| edit |st: &mut State| {
        if let Some(button) = MouseButton::ALL.get(i as usize) {
            st.set_action(Action::Mouse { button: *button });
        }
    });
    on!(on_set_media, |i| edit |st: &mut State| {
        if let Some(key) = keys::MEDIA.get(i as usize) {
            st.set_action(Action::Key { key: key.to_string(), mods: vec![] });
        }
    });
    on!(on_set_macro_choice, |i| edit |st: &mut State| {
        let Some(m) = st.cfg.macros.get(i as usize) else { return };
        let name = m.name.clone();
        match st.action_mut() {
            Some(Action::Macro { name: n, .. }) => *n = name,
            _ => st.set_action(Action::Macro { name, mode: MacroMode::Once, count: 1, gap_ms: 0 }),
        }
    });
    on!(on_set_macro_mode, |i| edit |st: &mut State| {
        if let Some(Action::Macro { mode, .. }) = st.action_mut() {
            *mode = MacroMode::ALL.get(i as usize).copied().unwrap_or_default();
        }
    });
    on!(on_set_macro_number, |field, value| edit |st: &mut State| {
        let Some(v) = parse_ms(&value) else { return };
        if let Some(Action::Macro { count, gap_ms, .. }) = st.action_mut() {
            match field.as_str() {
                "count" => *count = v.clamp(1, 10_000),
                _ => *gap_ms = v,
            }
        }
    });
    on!(on_new_macro_for_binding, | | app |a: &Rc<App>| a.new_macro(true));
    on!(on_set_turbo_output, |kind| edit |st: &mut State| {
        if let Some(Action::Turbo { output, .. }) = st.action_mut() {
            match (kind, &output) {
                (0, Output::Key { .. }) | (1, Output::Mouse { .. }) | (2, Output::Pad { .. }) => {}
                (1, _) => *output = Output::Mouse { button: MouseButton::Left },
                (2, _) => *output = Output::Pad { button: PadButton::A },
                _ => *output = Output::Key { key: "Space".into(), mods: vec![] },
            }
        }
    });
    on!(on_set_turbo_pad, |i| edit |st: &mut State| {
        if let (Some(Action::Turbo { output, .. }), Some(b)) = (st.action_mut(), PadButton::OUTPUTS.get(i as usize)) {
            *output = Output::Pad { button: *b };
        }
    });
    on!(on_set_pad_output, |i| edit |st: &mut State| {
        if let Some(button) = PadButton::OUTPUTS.get(i as usize) {
            st.set_action(Action::Pad { button: *button });
        }
    });
    on!(on_install_vpad_driver, | | app |a: &Rc<App>| a.install_vpad_driver());
    on!(on_set_turbo_mouse, |i| edit |st: &mut State| {
        if let (Some(Action::Turbo { output, .. }), Some(b)) = (st.action_mut(), MouseButton::ALL.get(i as usize)) {
            *output = Output::Mouse { button: *b };
        }
    });
    on!(on_set_turbo_number, |field, value| edit |st: &mut State| {
        let Some(v) = parse_ms(&value) else { return };
        if let Some(Action::Turbo { interval_ms, hold_ms, delay_ms, .. }) = st.action_mut() {
            match field.as_str() {
                "interval" => *interval_ms = v.max(engine::MIN_TURBO_INTERVAL_MS),
                "hold" => *hold_ms = v.max(1),
                _ => *delay_ms = v,
            }
        }
    });
    on!(on_set_turbo_toggle, |on| edit |st: &mut State| {
        if let Some(Action::Turbo { toggle, .. }) = st.action_mut() {
            *toggle = on;
        }
    });
    on!(on_set_text, |t| edit |st: &mut State| {
        if let Some(Action::Text { text, .. }) = st.action_mut() {
            *text = t.to_string();
        }
    });
    on!(on_set_text_delay, |v| edit |st: &mut State| {
        if let (Some(Action::Text { delay_ms, .. }), Some(v)) = (st.action_mut(), parse_ms(&v)) {
            *delay_ms = v.min(5000);
        }
    });
    on!(on_set_launch, |p, args_text| edit |st: &mut State| {
        if let Some(Action::Launch { path, args }) = st.action_mut() {
            *path = p.to_string();
            *args = args_text.to_string();
        }
    });
    {
        let a = app.clone();
        s.on_browse_launch(move || {
            let Some(file) = rfd::FileDialog::new()
                .set_title("Choose a program or file")
                .add_filter("Programs", &["exe", "bat", "cmd", "lnk"])
                .add_filter("All files", &["*"])
                .pick_file()
            else {
                return;
            };
            a.edit(|_, st| {
                if let Some(Action::Launch { path, .. }) = st.action_mut() {
                    *path = file.display().to_string();
                }
            });
        });
    }
    on!(on_set_sens, |v| edit |st: &mut State| {
        if let Some(Action::Sensitivity { speed }) = st.action_mut() {
            *speed = v.clamp(1, 20) as u32;
        }
    });

    // ---- macros ----------------------------------------------------------------------
    on!(on_select_macro, |i| nav |st: &mut State| {
        if (i as usize) < st.cfg.macros.len() {
            st.macro_idx = Some(i as usize);
            st.step = None;
            st.view = 1;
        }
    });
    on!(on_add_macro, | | app |a: &Rc<App>| a.new_macro(false));
    on!(on_duplicate_macro, | | edit |st: &mut State| {
        let Some(m) = st.current_macro().cloned() else { return };
        let name = st.cfg.unique_macro_name(&format!("{} copy", m.name));
        st.cfg.macros.push(Macro { name, ..m });
        st.macro_idx = Some(st.cfg.macros.len() - 1);
        st.step = None;
    });
    on!(on_delete_macro, | | edit |st: &mut State| {
        let Some(i) = st.macro_idx else { return };
        let m = st.cfg.macros.remove(i);
        st.cfg.retarget_macro(&m.name, None);
        st.step = None;
        if st.cfg.macros.is_empty() {
            st.macro_idx = None;
            st.view = 0;
        } else {
            st.macro_idx = Some(i.min(st.cfg.macros.len() - 1));
        }
    });
    on!(on_rename_macro, |name| edit |st: &mut State| {
        let Some(i) = st.macro_idx else { return };
        let name = name.trim().to_string();
        let old = st.cfg.macros[i].name.clone();
        if name.is_empty() || name == old {
            return;
        }
        let new = config::unique_name(&name, |n| st.cfg.macros.iter().enumerate().any(|(j, m)| j != i && m.name == n));
        st.cfg.macros[i].name = new.clone();
        st.cfg.retarget_macro(&old, Some(&new));
    });
    on!(on_set_macro_jitter, |v| edit |st: &mut State| {
        let Some(v) = parse_ms(&v) else { return };
        if let Some(m) = st.current_macro() {
            m.jitter_ms = v.min(10_000);
        }
    });
    on!(on_toggle_record, | | app |a: &Rc<App>| a.toggle_record());
    on!(on_clear_steps, | | edit |st: &mut State| {
        if let Some(m) = st.current_macro() {
            m.steps.clear();
        }
        st.step = None;
    });
    on!(on_remove_delays, | | edit |st: &mut State| {
        if let Some(m) = st.current_macro() {
            m.steps.retain(|s| !matches!(s, MacroStep::Delay { .. }));
        }
        st.step = None;
    });
    on!(on_set_all_delays, |v| edit |st: &mut State| {
        let Some(ms) = parse_ms(&v) else { return };
        let Some(m) = st.current_macro() else { return };
        if m.steps.iter().any(|s| matches!(s, MacroStep::Delay { .. })) {
            for s in &mut m.steps {
                if let MacroStep::Delay { ms: d } = s {
                    *d = ms;
                }
            }
        } else if m.steps.len() > 1 {
            // No delays yet: put one between every pair of steps.
            let steps = std::mem::take(&mut m.steps);
            let n = steps.len();
            for (i, s) in steps.into_iter().enumerate() {
                m.steps.push(s);
                if i + 1 < n {
                    m.steps.push(MacroStep::Delay { ms });
                }
            }
        }
        st.step = None;
    });
    {
        let a = app.clone();
        s.on_add_step(move |kind| {
            let mut is_key = false;
            a.edit(|_, st| {
                let at = st.step.map(|s| s + 1);
                let Some(m) = st.current_macro() else { return };
                let step = make_step(kind, "A".into(), MouseButton::Left, None, None);
                is_key = step_key(&step).is_some();
                let at = at.unwrap_or(m.steps.len()).min(m.steps.len());
                m.steps.insert(at, step);
                st.step = Some(at);
            });
            if is_key {
                // Ask for the key straight away.
                a.start_capture("step");
            }
        });
    }
    on!(on_select_step, |i| nav |st: &mut State| st.step = Some(i.max(0) as usize));
    on!(on_set_step_kind, |kind| edit |st: &mut State| {
        let Some(step) = st.current_step() else { return };
        let key = step_key(step).unwrap_or("A").to_string();
        let button = step_button(step).unwrap_or(MouseButton::Left);
        let ms = step_ms(step);
        let xy = step_xy(step);
        let text = match step {
            MacroStep::Text { text } => Some(text.clone()),
            _ => None,
        };
        let mut new = make_step(kind, key, button, ms, xy);
        if let (MacroStep::Text { text: t }, Some(old)) = (&mut new, text) {
            *t = old;
        }
        *step = new;
    });
    on!(on_set_step_mouse, |i| edit |st: &mut State| {
        let Some(b) = MouseButton::ALL.get(i as usize).copied() else { return };
        if let Some(MacroStep::MouseClick { button, .. } | MacroStep::MouseDown { button } | MacroStep::MouseUp { button }) = st.current_step() {
            *button = b
        }
    });
    on!(on_set_step_number, |v| edit |st: &mut State| {
        let Some(v) = parse_ms(&v) else { return };
        match st.current_step() {
            Some(MacroStep::KeyTap { hold_ms, .. } | MacroStep::MouseClick { hold_ms, .. }) => *hold_ms = v,
            Some(MacroStep::Delay { ms }) => *ms = v,
            _ => {}
        }
    });
    on!(on_set_step_text, |t| edit |st: &mut State| {
        if let Some(MacroStep::Text { text }) = st.current_step() {
            *text = t.to_string();
        }
    });
    on!(on_set_step_xy, |vx, vy| edit |st: &mut State| {
        if let Some(MacroStep::MouseMove { x, y, .. }) = st.current_step() {
            if let Some(v) = parse_coord(&vx) {
                *x = v;
            }
            if let Some(v) = parse_coord(&vy) {
                *y = v;
            }
        }
    });
    on!(on_pick_position, | | app |a: &Rc<App>| a.pick_position());
    on!(on_move_step, |delta| edit |st: &mut State| {
        let Some(i) = st.step else { return };
        let Some(m) = st.current_macro() else { return };
        let j = i as i64 + delta as i64;
        if j < 0 || j as usize >= m.steps.len() {
            return;
        }
        m.steps.swap(i, j as usize);
        st.step = Some(j as usize);
    });
    on!(on_duplicate_step, | | edit |st: &mut State| {
        let Some(i) = st.step else { return };
        let Some(m) = st.current_macro() else { return };
        if let Some(s) = m.steps.get(i).cloned() {
            m.steps.insert(i + 1, s);
            st.step = Some(i + 1);
        }
    });
    on!(on_delete_step, | | edit |st: &mut State| {
        let Some(i) = st.step else { return };
        let Some(m) = st.current_macro() else { return };
        if i < m.steps.len() {
            m.steps.remove(i);
        }
        let len = m.steps.len();
        st.step = if len == 0 { None } else { Some(i.min(len - 1)) };
    });

    // ---- settings ------------------------------------------------------------------
    on!(on_set_autostart, |i| app |a: &Rc<App>| a.set_autostart(Autostart::from_index(i), false));
    on!(on_claim_autostart, | | app |a: &Rc<App>| {
        let mode = a.st.borrow().autostart;
        a.set_autostart(mode, true);
    });
    on!(on_set_close_to_tray, |on| edit |st: &mut State| st.cfg.close_to_tray = on);
    on!(on_import_profiles, | | app |a: &Rc<App>| a.import());
    on!(on_export_profile, | | app |a: &Rc<App>| a.export(false));
    on!(on_export_all, | | app |a: &Rc<App>| a.export(true));
    on!(on_open_config_folder, | | app |_a: &Rc<App>| {
        if let Some(dir) = Config::path().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
            let _ = std::fs::create_dir_all(&dir);
            system::open(&dir.display().to_string());
        }
    });
    on!(on_open_link, |url| app |_a: &Rc<App>| {
        if url.starts_with("https://") {
            system::open(&url);
        }
    });
    on!(on_clear_hotkey, |i| edit |st: &mut State| *st.cfg.hotkeys.slot_mut(i.max(0) as usize) = None);
    on!(on_set_notifications, |on| edit |st: &mut State| st.cfg.notifications = on);
    on!(on_preview_notification, | | app |_a: &Rc<App>| osd::show("Profile: Default"));
    on!(on_set_check_updates, |on| app |a: &Rc<App>| {
        a.edit(|_, st| st.cfg.check_updates = on);
        if on {
            a.auto_check_update();
        }
    });
    on!(on_check_update, | | app |a: &Rc<App>| a.check_update(true));
    on!(on_install_update, | | app |a: &Rc<App>| a.install_update());
    on!(on_open_release_page, | | app |a: &Rc<App>| {
        let page = a.st.borrow().update.as_ref().map(|r| r.page.clone());
        if let Some(page) = page.filter(|p| p.starts_with("https://")) {
            system::open(&page);
        }
    });

    {
        let a = app.clone();
        s.on_toggle_enabled(move || {
            let on = !a.st.borrow().cfg.enabled;
            a.set_enabled(on);
        });
    }
    on!(on_auto_profile, | | app |a: &Rc<App>| a.force_profile(None));
    on!(on_undo, | | app |a: &Rc<App>| a.undo(false));
    on!(on_redo, | | app |a: &Rc<App>| a.undo(true));
    {
        let a = app.clone();
        s.on_set_theme(move |dark| {
            if let Some(ui) = a.ui() {
                ui.global::<Theme>().set_dark(dark);
            }
            let mut st = a.st.borrow_mut();
            st.cfg.theme = if dark { ThemeMode::Dark } else { ThemeMode::Light };
            st.cfg.save();
        });
    }
}

/// Inspector "Action" choice (`kind` is a kind constant).
fn set_kind(st: &mut State, kind: i32) {
    if st.input.is_empty() {
        return;
    }
    let current = st.action().map(kind_of);
    if current == Some(kind) {
        st.pending_kind = None;
        return;
    }
    // What a new turbo presses: the input's own key or button.
    let base = Combo::parse(&st.input)
        .map(|c| c.trigger)
        .unwrap_or_else(|| st.input.clone());
    let base = split_device(&base).0.to_string();
    match kind {
        K_DEFAULT => st.clear_action(),
        K_OFF => st.set_action(Action::Disabled),
        K_LAYER => st.set_action(Action::Layer {
            layer: 1,
            toggle: false,
        }),
        K_SENS => st.set_action(Action::Sensitivity { speed: 4 }),
        K_TAPHOLD => {
            if st.slot == 0 {
                // Keep what the input did as its tap action.
                let tap = st.binding().filter(|a| !a.has_slots()).cloned().map(Box::new);
                st.set_action(Action::TapHold {
                    tap,
                    hold: None,
                    double: None,
                    hold_ms: 200,
                    double_ms: 250,
                });
            }
        }
        K_GYRO => st.set_action(Action::Gyro { toggle: false }),
        K_GESTURE => {
            if st.slot == 0 {
                // Letting go without moving does what the button did.
                let tap = st
                    .binding()
                    .filter(|a| !a.has_slots())
                    .cloned()
                    .or_else(|| {
                        base.strip_prefix("mouse:")
                            .and_then(MouseButton::from_id)
                            .filter(|b| !b.is_wheel())
                            .map(|button| Action::Mouse { button })
                    })
                    .or_else(|| {
                        base.strip_prefix("key:").map(|k| Action::Key {
                            key: k.to_string(),
                            mods: vec![],
                        })
                    })
                    .map(Box::new);
                st.set_action(Action::Gesture {
                    tap,
                    up: None,
                    down: None,
                    left: None,
                    right: None,
                    distance: 40,
                });
            }
        }
        K_TURBO => {
            // Turbo the input's own key or button by default ("rapid-fire this").
            let output = if let Some(k) = base.strip_prefix("key:") {
                Output::Key {
                    key: k.to_string(),
                    mods: vec![],
                }
            } else {
                match base.strip_prefix("mouse:").and_then(MouseButton::from_id) {
                    Some(b) if !b.is_wheel() => Output::Mouse { button: b },
                    _ => Output::Mouse {
                        button: MouseButton::Left,
                    },
                }
            };
            st.set_action(Action::Turbo {
                output,
                interval_ms: 50,
                hold_ms: 20,
                delay_ms: 0,
                toggle: false,
            });
        }
        K_TEXT => st.set_action(Action::Text {
            text: String::new(),
            delay_ms: 5,
        }),
        K_LAUNCH => st.set_action(Action::Launch {
            path: String::new(),
            args: String::new(),
        }),
        K_MACRO => {
            if let Some(m) = st.cfg.macros.first() {
                let name = m.name.clone();
                st.set_action(Action::Macro {
                    name,
                    mode: MacroMode::Once,
                    count: 1,
                    gap_ms: 0,
                });
            } else {
                st.pending_kind = Some(kind);
            }
        }
        // Key, mouse, media, system: wait for the user to pick a value.
        _ => st.pending_kind = Some(kind),
    }
}
