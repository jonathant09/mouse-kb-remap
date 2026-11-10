//! UI wiring: owns the app state, handles every Slint callback and pushes
//! state back into the `Store` global (`refresh`).

use crate::config::{
    self, combo_label, per_second, Action, Config, Macro, MacroMode, MacroStep, Modifier, MouseButton, Output, Profile,
    ThemeMode,
};
use crate::engine::{self, Engine, EngineEvent, RecEvent};
use crate::system::{self, Autostart};
use crate::{keys, AppTray, AppWindow, BindingRow, KeyCap, MacroItem, MouseBtn, ProfileItem, StepRow, Store, Theme};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

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

// Action kinds, in the order of `Store.kind-names`.
const K_DEFAULT: i32 = 0;
const K_KEY: i32 = 1;
const K_MOUSE: i32 = 2;
const K_MACRO: i32 = 3;
const K_TURBO: i32 = 4;
const K_TEXT: i32 = 5;
const K_LAUNCH: i32 = 6;
const K_MEDIA: i32 = 7;
const K_SHIFT: i32 = 8;
const K_SENS: i32 = 9;
const K_OFF: i32 = 10;

/// Longest delay / hold accepted from a text field (1 hour).
const MAX_MS: u32 = 3_600_000;

struct State {
    cfg: Config,
    view: i32,
    /// Profile shown in the editor.
    profile: usize,
    /// 0 base layer, 1 shift layer.
    layer: usize,
    /// Selected input id (`key:A`, `mouse:back`) or empty.
    input: String,
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
    focus_exe: String,
    focus_self: bool,
    elevation_blocked: bool,
    recent: Vec<String>,
    autostart: Autostart,
    /// Exe the startup entry opens when it isn't this one (another copy).
    autostart_other: String,
    notice: (String, bool),
}

impl State {
    fn bindings(&self) -> &BTreeMap<String, Action> {
        self.cfg.profiles[self.profile].layer(self.layer)
    }
    fn bindings_mut(&mut self) -> &mut BTreeMap<String, Action> {
        let layer = self.layer;
        self.cfg.profiles[self.profile].layer_mut(layer)
    }
    fn action(&self) -> Option<&Action> {
        self.bindings().get(&self.input)
    }
    fn action_mut(&mut self) -> Option<&mut Action> {
        let input = self.input.clone();
        self.bindings_mut().get_mut(&input)
    }
    fn set_action(&mut self, action: Action) {
        if self.input.is_empty() {
            return;
        }
        let input = self.input.clone();
        self.bindings_mut().insert(input, action);
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
}

pub struct App {
    ui: slint::Weak<AppWindow>,
    /// Whether the window has been centred yet (it isn't shown at all when
    /// started with --tray).
    centred: std::cell::Cell<bool>,
    tray: RefCell<Option<AppTray>>,
    engine: Engine,
    st: RefCell<State>,
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

pub fn run(start_hidden: bool) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    system::stop_raw_keyboard_input();
    let cfg = Config::load();
    ui.global::<Theme>().set_dark(cfg.theme == ThemeMode::Dark);

    // The engine thread reports focus changes and captured keys; hop onto
    // the UI thread.
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
            input: String::new(),
            pending_kind: None,
            macro_idx: None,
            step: None,
            capturing: String::new(),
            ui_lone_mod: None,
            recording: false,
            record_target: String::new(),
            focus_exe: String::new(),
            focus_self: true,
            elevation_blocked: false,
            recent: Vec::new(),
            autostart: Autostart::Off,
            autostart_other: String::new(),
            notice: (String::new(), false),
        }),
        engine,
    });
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    let store = ui.global::<Store>();
    store.set_version(env!("CARGO_PKG_VERSION").into());
    store.set_engine_supported(app.engine.supported());
    store.set_self_elevated(app.engine.elevated());
    store.set_autostart_supported(system::autostart_supported());
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
    wire_callbacks(&ui, &app);

    // Tray icon + close-to-tray.
    let tray = AppTray::new()?;
    tray.on_show_window(|| with_app(|app| app.show_window()));
    tray.on_toggle_enabled(|| with_app(|app| app.edit(|_, st| st.cfg.enabled = !st.cfg.enabled)));
    tray.on_quit(|| with_app(|app| app.quit()));
    *app.tray.borrow_mut() = Some(tray);

    ui.window().on_close_requested(|| {
        let mut response = slint::CloseRequestResponse::HideWindow;
        with_app(|app| {
            if !app.st.borrow().cfg.close_to_tray {
                app.quit();
            } else {
                app.cancel_capture();
            }
            response = slint::CloseRequestResponse::HideWindow;
        });
        response
    });

    app.refresh();
    app.check_autostart();
    if !start_hidden {
        app.show_window();
    }
    slint::run_event_loop_until_quit()?;
    app.engine.shutdown();
    Ok(())
}

/// Centre the window in the work area, shrinking it if the screen is small.
/// (The default position can push the bottom off screen at high scaling.)
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

fn kind_of(action: &Action) -> i32 {
    match action {
        Action::Key { key, .. } if keys::MEDIA.contains(&key.as_str()) => K_MEDIA,
        Action::Key { .. } => K_KEY,
        Action::Mouse { .. } => K_MOUSE,
        Action::Macro { .. } => K_MACRO,
        Action::Turbo { .. } => K_TURBO,
        Action::Text { .. } => K_TEXT,
        Action::Launch { .. } => K_LAUNCH,
        Action::Shift => K_SHIFT,
        Action::Sensitivity { .. } => K_SENS,
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

fn make_step(kind: i32, key: String, button: MouseButton, ms: Option<u32>) -> MacroStep {
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
        _ => MacroStep::Text { text: String::new() },
    }
}

/// Input ids look like `key:A`, `key:F1`, `mouse:back`.
fn input_label(input: &str) -> String {
    if let Some(k) = input.strip_prefix("key:") {
        keys::display_name(k)
    } else if let Some(m) = input.strip_prefix("mouse:") {
        MouseButton::from_id(m)
            .map(|b| b.name().to_string())
            .unwrap_or_else(|| m.to_string())
    } else {
        input.to_string()
    }
}

fn input_detail(input: &str) -> String {
    if let Some(k) = input.strip_prefix("key:") {
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

/// What an unbound input in this profile/layer does instead, as a sentence.
fn inherited(cfg: &Config, profile: usize, layer: usize, input: &str) -> Option<String> {
    let def = &cfg.profiles[0];
    let p = &cfg.profiles[profile];
    if layer == 1 {
        if profile > 0 {
            if let Some(a) = def.shift_bindings.get(input) {
                return Some(format!("Uses the Default profile's shift layer: {}.", a.label()));
            }
        }
        if let Some(a) = p.bindings.get(input) {
            return Some(format!(
                "Nothing on the shift layer, so the base binding applies: {}.",
                a.label()
            ));
        }
        if profile > 0 {
            if let Some(a) = def.bindings.get(input) {
                return Some(format!(
                    "Nothing on the shift layer, so Default's binding applies: {}.",
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
    if layer == 1 {
        let a = (profile > 0)
            .then(|| def.shift_bindings.get(input))
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

impl App {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// Change config: then save, push to the engine and redraw.
    fn edit(&self, f: impl FnOnce(&App, &mut State)) {
        {
            let mut st = self.st.borrow_mut();
            f(self, &mut st);
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
        }
        self.refresh();
    }

    fn notice(&self, text: impl Into<String>, error: bool) {
        self.st.borrow_mut().notice = (text.into(), error);
        self.refresh();
    }

    pub fn show_window(&self) {
        let Some(ui) = self.ui() else { return };
        let _ = ui.show();
        ui.window().set_minimized(false);
        if !self.centred.replace(true) {
            // The real size is only known once the window exists.
            let weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
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
        if target == "binding" && self.st.borrow().input.is_empty() {
            return;
        }
        // Engine capture where possible; the window's FocusScope otherwise.
        let _ = self.engine.start_capture();
        self.nav(|_, st| st.capturing = target.to_string());
    }

    fn on_engine_event(&self, ev: EngineEvent) {
        match ev {
            EngineEvent::Focus(info) => self.nav(|_, st| {
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
            }),
            EngineEvent::Captured { vk, mods } => {
                if let Some(k) = keys::find_by_vk(vk) {
                    self.captured(k.id, mods);
                } else {
                    // A key we don't know: listen for another one.
                    let _ = self.engine.start_capture();
                }
            }
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
                    st.input = format!("key:{key}");
                    st.pending_kind = None;
                    if keys::find(&key).is_some_and(|k| k.group == keys::Group::Numpad) {
                        st.cfg.full_keyboard = true;
                    }
                });
            }
            _ => self.refresh(),
        }
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
            st.cfg.macros.push(Macro {
                name: name.clone(),
                steps: vec![],
            });
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
        let live = cfg.profile_for_exe(&st.focus_exe);
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
                    live: i == live && !st.focus_exe.is_empty(),
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

        // Header
        s.set_profile_name(profile.name.clone().into());
        s.set_profile_apps(model(profile.apps.iter().map(SharedString::from).collect()));
        s.set_recent_apps(model(
            st.recent
                .iter()
                .filter(|r| !profile.apps.iter().any(|a| a.eq_ignore_ascii_case(r)))
                .take(4)
                .map(SharedString::from)
                .collect(),
        ));
        s.set_layer(st.layer as i32);
        // Inputs that hold the shift layer in this profile: its own, plus
        // Default's that it doesn't override.
        let inherited_shift = cfg.profiles[0]
            .bindings
            .iter()
            .filter(|(i, _)| st.profile > 0 && !profile.bindings.contains_key(*i));
        let shift_keys: Vec<String> = profile
            .bindings
            .iter()
            .chain(inherited_shift)
            .filter(|(_, a)| matches!(a, Action::Shift))
            .map(|(i, _)| input_label(i))
            .collect();
        s.set_shift_hint(
            if shift_keys.is_empty() {
                "no input is set to Shift layer yet".to_string()
            } else {
                format!("hold {} to use", shift_keys.join(" or "))
            }
            .into(),
        );
        s.set_full_keyboard(cfg.full_keyboard);

        // Keyboard
        s.set_keys(model(
            keys::layout(cfg.full_keyboard)
                .map(|k| {
                    let id = format!("key:{}", k.id);
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
                    let id = format!("mouse:{m}");
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
                        label: input_label(&id).into(),
                        detail: input_detail(&id).into(),
                        value: value.into(),
                        mapped: action.is_some(),
                        supported: true,
                    }
                })
                .collect(),
        ));

        // All bindings of this profile (base first, mouse before keys)
        let mut rows: Vec<BindingRow> = [0usize, 1]
            .iter()
            .flat_map(|&layer| {
                profile.layer(layer).iter().map(move |(input, action)| BindingRow {
                    input_id: input.clone().into(),
                    input: input_label(input).into(),
                    action: action.label().into(),
                    layer: layer as i32,
                })
            })
            .collect();
        rows.sort_by_key(|r| (r.layer, !r.input_id.starts_with("mouse:")));
        s.set_bindings(model(rows));

        // Inspector
        let input = st.input.as_str();
        s.set_selected_input(input.into());
        s.set_selected_label(input_label(input).into());
        s.set_selected_detail(
            if input == "mouse:dpi" {
                "Handled by firmware".into()
            } else {
                input_detail(input)
            }
            .into(),
        );
        s.set_selected_supported(input != "mouse:dpi");
        let action = st.action();
        s.set_selected_kind(
            st.pending_kind
                .unwrap_or_else(|| action.map(kind_of).unwrap_or(K_DEFAULT)),
        );
        s.set_inherited_value(inherited(cfg, st.profile, st.layer, input).unwrap_or_default().into());

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
                    s.set_turbo_is_mouse(false);
                    s.set_turbo_key(combo_label(key, mods).into());
                    s.set_turbo_mods(Modifier::to_bits(mods) as i32);
                    s.set_turbo_mouse(-1);
                }
                Output::Mouse { button } => {
                    s.set_turbo_is_mouse(true);
                    s.set_turbo_key("".into());
                    s.set_turbo_mods(0);
                    s.set_turbo_mouse(MouseButton::ALL.iter().position(|b| b == button).unwrap_or(0) as i32);
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

        // Macro editor
        let m = st.macro_idx.and_then(|i| cfg.macros.get(i));
        s.set_macro_name(m.map(|m| m.name.clone()).unwrap_or_default().into());
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
        } else {
            s.set_step_kind(-1);
        }

        // Settings
        s.set_autostart(st.autostart.index());
        s.set_autostart_other(st.autostart_other.clone().into());
        s.set_close_to_tray(cfg.close_to_tray);
        s.set_notice(st.notice.0.clone().into());
        s.set_notice_error(st.notice.1);

        // Status bar
        s.set_focused_app(st.focus_exe.clone().into());
        let live_name = cfg.profiles[live].name.clone();
        s.set_live_profile(
            if st.focus_self && !st.focus_exe.is_empty() {
                format!("{live_name} (paused while this window is focused)")
            } else {
                live_name.clone()
            }
            .into(),
        );
        s.set_elevation_warning(st.elevation_blocked && !st.focus_self);
        s.set_engine_enabled(cfg.enabled);

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
    on!(on_delete_profile, | | edit |st: &mut State| {
        if st.profile > 0 {
            st.cfg.profiles.remove(st.profile);
            st.profile -= 1;
            st.pending_kind = None;
        }
    });
    {
        let a = app.clone();
        s.on_rename_profile(move |name| {
            let mut st = a.st.borrow_mut();
            let name = name.trim();
            if st.profile == 0 || name.is_empty() {
                return;
            }
            let p = st.profile;
            st.cfg.profiles[p].name = name.to_string();
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
        if !name.contains('.') {
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
    on!(on_select_layer, |layer| nav |st: &mut State| {
        st.layer = layer.clamp(0, 1) as usize;
        st.pending_kind = None;
    });
    on!(on_set_full_keyboard, |on| edit |st: &mut State| st.cfg.full_keyboard = on);

    // ---- bindings ----------------------------------------------------------------------
    on!(on_select_input, |id| nav |st: &mut State| {
        st.input = id.to_string();
        st.pending_kind = None;
    });
    {
        let a = app.clone();
        s.on_select_binding(move |id, layer| {
            if let Some(ui) = a.ui() {
                ui.global::<Store>().set_inspector_tab(0);
                if let Some(key) = id.strip_prefix("key:") {
                    let on_drawing = keys::find(key).is_some_and(|k| k.group != keys::Group::Extra);
                    if on_drawing {
                        ui.global::<Store>().set_device_tab(1);
                    }
                } else {
                    ui.global::<Store>().set_device_tab(0);
                }
            }
            a.nav(|_, st| {
                st.input = id.to_string();
                st.layer = layer.clamp(0, 1) as usize;
                st.pending_kind = None;
            });
        });
    }
    on!(on_clear_binding, |id, layer| edit |st: &mut State| {
        let p = st.profile;
        st.cfg.profiles[p].layer_mut(layer.clamp(0, 1) as usize).remove(id.as_str());
        st.pending_kind = None;
    });
    {
        let a = app.clone();
        s.on_set_kind(move |kind| {
            a.cancel_capture();
            a.edit(|_, st| set_kind(st, kind));
        });
    }
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
    on!(on_set_turbo_output, |mouse| edit |st: &mut State| {
        if let Some(Action::Turbo { output, .. }) = st.action_mut() {
            match (mouse, &output) {
                (true, Output::Key { .. }) => *output = Output::Mouse { button: MouseButton::Left },
                (false, Output::Mouse { .. }) => *output = Output::Key { key: "Space".into(), mods: vec![] },
                _ => {}
            }
        }
    });
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
        st.cfg.macros.push(Macro { name, steps: m.steps });
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
                let step = make_step(kind, "A".into(), MouseButton::Left, None);
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
        let text = match step {
            MacroStep::Text { text } => Some(text.clone()),
            _ => None,
        };
        let mut new = make_step(kind, key, button, ms);
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

    on!(on_toggle_enabled, | | edit |st: &mut State| st.cfg.enabled = !st.cfg.enabled);
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

/// Inspector "Action" choice.
fn set_kind(st: &mut State, kind: i32) {
    if st.input.is_empty() {
        return;
    }
    let current = st.action().map(kind_of);
    if current == Some(kind) {
        st.pending_kind = None;
        return;
    }
    let input = st.input.clone();
    match kind {
        K_DEFAULT => {
            st.bindings_mut().remove(&input);
            st.pending_kind = None;
        }
        K_OFF => st.set_action(Action::Disabled),
        K_SHIFT => {
            if st.layer == 0 {
                st.set_action(Action::Shift);
            }
        }
        K_SENS => st.set_action(Action::Sensitivity { speed: 4 }),
        K_TURBO => {
            // Turbo the input's own key or button by default ("rapid-fire this").
            let output = if let Some(k) = input.strip_prefix("key:") {
                Output::Key {
                    key: k.to_string(),
                    mods: vec![],
                }
            } else {
                match input.strip_prefix("mouse:").and_then(MouseButton::from_id) {
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
        // Key, mouse, media: wait for the user to pick a value.
        _ => st.pending_kind = Some(kind),
    }
}
