// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod engine;
mod keys;

use config::{input_detail, input_label, Action, Config, MouseButton, Profile, ThemeMode};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

slint::include_modules!();

/// Order of buttons expected by `MouseDiagram` in ui/mouse.slint.
const MOUSE_ORDER: [&str; 8] = [
    "left",
    "right",
    "middle",
    "wheel_up",
    "wheel_down",
    "back",
    "forward",
    "dpi",
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

struct State {
    cfg: Config,
    /// Profile shown in the editor.
    profile: usize,
    /// Selected input id (`key:A`, `mouse:back`) or empty.
    input: String,
    /// Action kind chosen in the inspector before a target was picked.
    pending_kind: Option<i32>,
    /// Last focused window that isn't ours.
    focus_exe: String,
    focus_self: bool,
    elevation_blocked: bool,
    recent: Vec<String>,
}

impl State {
    fn bindings(&self) -> &BTreeMap<String, Action> {
        &self.cfg.profiles[self.profile].bindings
    }
    fn bindings_mut(&mut self) -> &mut BTreeMap<String, Action> {
        &mut self.cfg.profiles[self.profile].bindings
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let cfg = Config::load();
    ui.global::<Theme>().set_dark(cfg.theme == ThemeMode::Dark);

    // The engine thread reports focus changes; hop onto the UI thread.
    let weak = ui.as_weak();
    let engine = Rc::new(engine::Engine::start(&cfg, move |info| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_engine_focus(info.exe.into(), info.is_self, info.elevation_blocked);
            }
        });
    }));

    let state = Rc::new(RefCell::new(State {
        cfg,
        profile: 0,
        input: String::new(),
        pending_kind: None,
        focus_exe: String::new(),
        focus_self: true,
        elevation_blocked: false,
        recent: Vec::new(),
    }));

    ui.set_engine_supported(engine.supported());
    ui.set_self_elevated(engine.elevated());
    ui.set_config_path(
        Config::path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    refresh(&ui, &state.borrow());

    // Small helper: run `f` on the state, then save / push to engine / redraw.
    let wire = |f: Box<dyn Fn(&mut State)>, persist: bool| {
        let ui = ui.as_weak();
        let state = state.clone();
        let engine = engine.clone();
        move || {
            let ui = ui.unwrap();
            let mut st = state.borrow_mut();
            f(&mut st);
            if persist {
                st.cfg.save();
                engine.apply(&st.cfg);
            }
            refresh(&ui, &st);
        }
    };

    // ---- profiles --------------------------------------------------------------
    {
        let (state, ui_w) = (state.clone(), ui.as_weak());
        ui.on_select_profile(move |i| {
            let mut st = state.borrow_mut();
            st.profile = (i.max(0) as usize).min(st.cfg.profiles.len() - 1);
            st.pending_kind = None;
            refresh(&ui_w.unwrap(), &st);
        });
    }
    ui.on_add_profile(wire(
        Box::new(|st| {
            let n = st.cfg.profiles.len();
            st.cfg.profiles.push(Profile {
                name: format!("New profile {n}"),
                apps: vec![],
                bindings: BTreeMap::new(),
            });
            st.profile = n;
            st.pending_kind = None;
        }),
        true,
    ));
    ui.on_delete_profile(wire(
        Box::new(|st| {
            if st.profile > 0 {
                st.cfg.profiles.remove(st.profile);
                st.profile -= 1;
                st.pending_kind = None;
            }
        }),
        true,
    ));
    {
        let (state, ui_w) = (state.clone(), ui.as_weak());
        ui.on_rename_profile(move |name| {
            let mut st = state.borrow_mut();
            let name = name.trim();
            if st.profile == 0 || name.is_empty() {
                return;
            }
            let p = st.profile;
            st.cfg.profiles[p].name = name.to_string();
            st.cfg.save();
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_add_app(move |app| {
            let mut st = state.borrow_mut();
            let mut app = app.trim().to_string();
            if app.is_empty() || st.profile == 0 {
                return;
            }
            if !app.contains('.') {
                app.push_str(".exe");
            }
            let p = st.profile;
            let apps = &mut st.cfg.profiles[p].apps;
            if !apps.iter().any(|a| a.eq_ignore_ascii_case(&app)) {
                apps.push(app);
            }
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_remove_app(move |app| {
            let mut st = state.borrow_mut();
            let p = st.profile;
            st.cfg.profiles[p].apps.retain(|a| a.as_str() != app.as_str());
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }

    // ---- bindings ----------------------------------------------------------------
    {
        let (state, ui_w) = (state.clone(), ui.as_weak());
        ui.on_select_input(move |id| {
            let mut st = state.borrow_mut();
            st.input = id.to_string();
            st.pending_kind = None;
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_set_kind(move |kind| {
            let mut st = state.borrow_mut();
            if st.input.is_empty() {
                return;
            }
            let input = st.input.clone();
            match kind {
                0 => {
                    st.bindings_mut().remove(&input);
                    st.pending_kind = None;
                }
                3 => {
                    st.bindings_mut().insert(input, Action::Disabled);
                    st.pending_kind = None;
                }
                k => {
                    let current = st.bindings().get(&input).map(kind_of);
                    st.pending_kind = if current == Some(k) { None } else { Some(k) };
                }
            }
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_key_captured(move |text| {
            let Some(key) = keys::from_slint_text(&text) else {
                return;
            };
            let mut st = state.borrow_mut();
            if st.input.is_empty() {
                return;
            }
            let input = st.input.clone();
            st.bindings_mut().insert(input, Action::Key { key: key.to_string() });
            st.pending_kind = None;
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_set_mouse_target(move |id| {
            let Some(button) = MouseButton::from_id(&id) else {
                return;
            };
            let mut st = state.borrow_mut();
            if st.input.is_empty() {
                return;
            }
            let input = st.input.clone();
            st.bindings_mut().insert(input, Action::Mouse { button });
            st.pending_kind = None;
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }
    {
        let (state, engine, ui_w) = (state.clone(), engine.clone(), ui.as_weak());
        ui.on_clear_binding(move |id| {
            let mut st = state.borrow_mut();
            st.bindings_mut().remove(id.as_str());
            st.pending_kind = None;
            st.cfg.save();
            engine.apply(&st.cfg);
            refresh(&ui_w.unwrap(), &st);
        });
    }

    // ---- app-level -----------------------------------------------------------------
    ui.on_toggle_enabled(wire(Box::new(|st| st.cfg.enabled = !st.cfg.enabled), true));
    {
        let (state, ui_w) = (state.clone(), ui.as_weak());
        ui.on_set_theme(move |dark| {
            let ui = ui_w.unwrap();
            ui.global::<Theme>().set_dark(dark);
            let mut st = state.borrow_mut();
            st.cfg.theme = if dark { ThemeMode::Dark } else { ThemeMode::Light };
            st.cfg.save();
        });
    }
    {
        let (state, ui_w) = (state.clone(), ui.as_weak());
        ui.on_engine_focus(move |exe, is_self, blocked| {
            let mut st = state.borrow_mut();
            st.focus_self = is_self;
            if !is_self && !exe.is_empty() {
                let exe = exe.to_string();
                st.elevation_blocked = blocked;
                if !IGNORED_APPS.contains(&exe.to_ascii_lowercase().as_str()) {
                    st.recent.retain(|a| !a.eq_ignore_ascii_case(&exe));
                    st.recent.insert(0, exe.clone());
                    st.recent.truncate(6);
                }
                st.focus_exe = exe;
            }
            refresh(&ui_w.unwrap(), &st);
        });
    }

    ui.run()
}

fn kind_of(action: &Action) -> i32 {
    match action {
        Action::Key { .. } => 1,
        Action::Mouse { .. } => 2,
        Action::Disabled => 3,
    }
}

/// Very short label used on keycaps.
fn short_label(action: &Action) -> String {
    match action {
        Action::Key { key } => match keys::find(key) {
            Some(k) if !k.label.is_empty() => k.label.to_string(),
            _ => key.clone(),
        },
        Action::Mouse { button } => match button {
            MouseButton::Left => "LMB",
            MouseButton::Right => "RMB",
            MouseButton::Middle => "MMB",
            MouseButton::Back => "M4",
            MouseButton::Forward => "M5",
            MouseButton::WheelUp => "Wh↑",
            MouseButton::WheelDown => "Wh↓",
        }
        .to_string(),
        Action::Disabled => "Off".to_string(),
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// Push the whole state into the UI. Cheap enough (a few hundred small
/// structs) that we don't bother with fine-grained updates.
fn refresh(ui: &AppWindow, st: &State) {
    let cfg = &st.cfg;
    let live = cfg.profile_for_exe(&st.focus_exe);
    let bindings = st.bindings();
    let defaults = &cfg.profiles[0].bindings;

    // Sidebar
    ui.set_profiles(model(
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
                count: p.bindings.len() as i32,
            })
            .collect(),
    ));
    ui.set_selected_profile(st.profile as i32);

    // Header
    let profile = &cfg.profiles[st.profile];
    ui.set_profile_name(profile.name.clone().into());
    ui.set_profile_apps(model(profile.apps.iter().map(SharedString::from).collect()));
    ui.set_recent_apps(model(
        st.recent
            .iter()
            .filter(|r| !profile.apps.iter().any(|a| a.eq_ignore_ascii_case(r)))
            .take(4)
            .map(SharedString::from)
            .collect(),
    ));

    // Keyboard
    ui.set_keys(model(
        keys::KEYS
            .iter()
            .map(|k| {
                let id = format!("key:{}", k.id);
                let action = bindings.get(&id);
                KeyCap {
                    id: id.into(),
                    label: k.label.into(),
                    x: k.x,
                    y: k.y,
                    w: k.w,
                    mapped: action.is_some(),
                    value: action.map(short_label).unwrap_or_default().into(),
                }
            })
            .collect(),
    ));

    // Mouse
    ui.set_mouse_buttons(model(
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
                let value = match (action, st.profile > 0, defaults.get(&id)) {
                    (Some(a), _, _) => a.label(),
                    (None, true, Some(d)) => format!("{} (Default)", d.label()),
                    _ => "Default".to_string(),
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

    // Bindings list (mouse first, then keys)
    let mut rows: Vec<BindingRow> = bindings
        .iter()
        .map(|(input, action)| BindingRow {
            input_id: input.clone().into(),
            input: input_label(input).into(),
            action: action.label().into(),
        })
        .collect();
    rows.sort_by_key(|r| !r.input_id.starts_with("mouse:"));
    ui.set_bindings(model(rows));

    // Inspector
    let input = st.input.as_str();
    ui.set_selected_input(input.into());
    ui.set_selected_label(input_label(input).into());
    ui.set_selected_detail(if input == "mouse:dpi" {
        "Handled by firmware".into()
    } else {
        input_detail(input).into()
    });
    ui.set_selected_supported(input != "mouse:dpi");
    let action = bindings.get(input);
    ui.set_selected_kind(st.pending_kind.unwrap_or_else(|| action.map(kind_of).unwrap_or(0)));
    ui.set_selected_value(
        match action {
            Some(Action::Key { key }) => keys::display_name(key),
            Some(Action::Mouse { button }) => button.id().to_string(),
            _ => String::new(),
        }
        .into(),
    );
    ui.set_inherited_value(
        if st.profile > 0 {
            defaults.get(input).map(|a| a.label()).unwrap_or_default()
        } else {
            String::new()
        }
        .into(),
    );

    // Status bar
    ui.set_focused_app(st.focus_exe.clone().into());
    let live_name = cfg.profiles[live].name.clone();
    ui.set_live_profile(
        if st.focus_self && !st.focus_exe.is_empty() {
            format!("{live_name} (paused while this window is focused)")
        } else {
            live_name
        }
        .into(),
    );
    ui.set_elevation_warning(st.elevation_blocked && !st.focus_self);
    ui.set_engine_enabled(cfg.enabled);
}
