//! Every page fits the smallest window.
//!
//! The window can't be made smaller than 1080 x 660 (`min-width` /
//! `min-height` in ui/app.slint). If a row in some view needs more width
//! than it gets, Slint lays it out wider than the window anyway, and
//! whatever is right of it (the inspector, buttons) ends up cut off at the
//! window edge. This test fills the real UI (through `fill_ui`, the same
//! code the app uses) with crowded data, walks through every page and
//! inspector state at the minimum size, and fails if any text, text field
//! or clickable area reaches past the right edge of the window or of the
//! scroll area it sits in.
//!
//! A new page or panel is covered as soon as one of the states below
//! shows it; add a state here when adding a view that none of them reach.

use super::*;
use crate::config::{Combo, Macro, MacroStep, Modifier, MouseButton, PadButton};
use i_slint_backend_testing::{ElementHandle, ElementQuery};
use slint::{ComponentHandle, LogicalSize};

const MIN_W: f32 = 1080.0;
const MIN_H: f32 = 660.0;
/// Rounding slack (layout positions are fractional).
const SLACK: f32 = 0.5;

fn crowded_config() -> Config {
    let mut cfg = Config::default();
    cfg.profiles[1].apps = vec![
        "GenshinImpact.exe".into(),
        "YuanShen.exe".into(),
        "title:Genshin Impact launcher".into(),
    ];
    let mut long = Profile::new("A profile with a fairly long name for testing");
    long.apps
        .push("SomeGameWithALongExecutableName-Win64-Shipping.exe".into());
    cfg.profiles.push(long);
    cfg.devices = vec![
        Device {
            name: "Trackball".into(),
            id: "\\\\?\\HID#VID_046D&PID_C539#1".into(),
        },
        Device {
            name: "Razer Viper V3 Pro".into(),
            id: "\\\\?\\HID#VID_1532&PID_00C0#2".into(),
        },
    ];
    let mut steps = Vec::new();
    for _ in 0..12 {
        steps.push(MacroStep::KeyTap {
            key: "F".into(),
            hold_ms: 30,
        });
        steps.push(MacroStep::Delay { ms: 68 });
        steps.push(MacroStep::MouseClick {
            button: MouseButton::Left,
            hold_ms: 20,
        });
        steps.push(MacroStep::Text {
            text: "A fairly long piece of text typed by the macro".into(),
        });
        steps.push(MacroStep::MouseMove {
            x: 1919,
            y: 1079,
            absolute: true,
        });
        steps.push(MacroStep::PadTap {
            button: PadButton::RsRight,
            hold_ms: 50,
        });
    }
    cfg.macros = vec![
        Macro::new("Genshin F with a rather long macro name", steps.clone()),
        Macro::new("Genshin Space", steps),
    ];
    cfg
}

fn crowded_state() -> State {
    let mut st = State::new(crowded_config());
    st.recent = [
        "chrome.exe",
        "Discord.exe",
        "explorer.exe",
        "WindowsTerminal.exe",
        "steamwebhelper.exe",
    ]
    .map(String::from)
    .to_vec();
    st.focus_exe = "SomeGameWithALongExecutableName-Win64-Shipping.exe".into();
    st.focus_self = false;
    st.elevation_blocked = true;
    st.forced = true;
    st.autostart_other = "C:\\Users\\someone\\Documents\\mouse-kb-remap\\mouse-kb-remap.exe".into();
    st.notice = (
        "Exported 3 profiles to C:\\Users\\someone\\Documents\\profiles.json".into(),
        false,
    );
    st.update_status = "Version 9.9.9 is available.".into();
    st
}

fn inherits(e: &ElementHandle, name: &str) -> bool {
    e.type_name().is_some_and(|t| t == name) || e.bases().is_some_and(|mut b| b.any(|t| t == name))
}

/// Elements a person reads or clicks: these must stay on screen.
fn is_content(e: &ElementHandle) -> bool {
    ["Text", "TextInput", "TouchArea"].iter().any(|t| inherits(e, t))
}

fn describe(e: &ElementHandle) -> String {
    let name = e
        .accessible_label()
        .filter(|l| !l.is_empty())
        .map(|l| format!("\"{l}\""))
        .or_else(|| e.id().map(|i| i.to_string()))
        .or_else(|| e.type_name().map(|t| t.to_string()))
        .unwrap_or_default();
    let p = e.absolute_position();
    let s = e.size();
    format!("{name} at x {:.0}..{:.0}", p.x, p.x + s.width)
}

/// What sticks out of the window, or out of the scroll area it is in.
fn overflow(ui: &AppWindow) -> Vec<String> {
    let mut bad = Vec::new();
    let all = ElementQuery::from_root(ui).match_descendants().find_all();
    // Guards against checking nothing (element queries need the Slint debug
    // info that build.rs turns on for debug builds).
    assert!(
        all.iter().filter(|e| is_content(e)).count() > 50,
        "no UI elements found"
    );
    for e in all.iter().filter(|e| is_content(e)) {
        let (p, s) = (e.absolute_position(), e.size());
        if s.width <= 0.0 {
            continue;
        }
        if p.x < -SLACK || p.x + s.width > MIN_W + SLACK {
            bad.push(format!("{} (window is 0..{MIN_W})", describe(e)));
        }
    }
    for f in all.iter().filter(|e| inherits(e, "Flickable")) {
        let (fp, fs) = (f.absolute_position(), f.size());
        for e in f.query_descendants().find_all().iter().filter(|e| is_content(e)) {
            let (p, s) = (e.absolute_position(), e.size());
            if s.width > 0.0 && (p.x < fp.x - SLACK || p.x + s.width > fp.x + fs.width + SLACK) {
                bad.push(format!(
                    "{} (scroll area is {:.0}..{:.0})",
                    describe(e),
                    fp.x,
                    fp.x + fs.width
                ));
            }
        }
    }
    bad.sort();
    bad.dedup();
    bad
}

struct Fit {
    ui: AppWindow,
    failures: Vec<String>,
    checked: usize,
}

impl Fit {
    fn check(&mut self, st: &State, what: &str) {
        fill_ui(&self.ui, st);
        self.checked += 1;
        for b in overflow(&self.ui) {
            self.failures.push(format!("{what}: {b}"));
        }
    }
}

#[test]
// Text is measured with the system's fonts; the layout is made for Windows'
// (Segoe UI). Other systems' fallback fonts are wider.
#[cfg_attr(not(windows), ignore = "measures text with Windows' fonts")]
fn every_page_fits_the_smallest_window() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = AppWindow::new().unwrap();
    ui.window().set_size(LogicalSize::new(MIN_W, MIN_H));
    ui.show().unwrap();
    let store = ui.global::<Store>();
    let mut fit = Fit {
        ui: ui.clone_strong(),
        failures: Vec::new(),
        checked: 0,
    };

    let mut st = crowded_state();

    // Profile editor: every profile, device tab, layer and device choice.
    st.view = 0;
    for profile in 0..st.cfg.profiles.len() {
        st.profile = profile;
        for tab in 0..3 {
            store.set_device_tab(tab);
            for full in [false, true] {
                st.cfg.full_keyboard = full;
                for layer in [0, 2] {
                    st.layer = layer;
                    for device in [0, 2] {
                        st.device = device;
                        st.select_input("");
                        fit.check(
                            &st,
                            &format!("profile {profile}, tab {tab}, numpad {full}, layer {layer}, device {device}"),
                        );
                    }
                }
            }
        }
    }
    st.layer = 0;
    st.device = 0;
    store.set_device_tab(0);

    // Inspector: every action kind, on a non-default profile (more header rows).
    for profile in [0, 1] {
        st.profile = profile;
        for kind in K_DEFAULT..=K_OFF {
            st.select_input("mouse:back");
            set_kind(&mut st, kind);
            fit.check(&st, &format!("profile {profile}, action kind {kind}"));
            if kind == K_KEY {
                for target in ["binding", "combo"] {
                    st.picker_target = target.into();
                    fit.check(&st, &format!("profile {profile}, key list open ({target})"));
                }
                st.picker_target.clear();
            }
            if kind == K_TURBO {
                for turbo in 0..3 {
                    store.set_turbo_kind(turbo);
                    fit.check(&st, &format!("profile {profile}, turbo output {turbo}"));
                }
            }
            if kind == K_TAPHOLD {
                for slot in 1..=3 {
                    st.slot = slot;
                    fit.check(&st, &format!("profile {profile}, tap / hold part {slot}"));
                }
            }
            if kind == K_GESTURE {
                for slot in 1..=5 {
                    st.slot = slot;
                    fit.check(&st, &format!("profile {profile}, gesture part {slot}"));
                }
            }
        }
        // A combo on that input, and the list of all bindings.
        let combo = Combo {
            mods: vec![Modifier::ALL[0]],
            held: vec![],
            trigger: "mouse:back".into(),
        }
        .id();
        st.select_input(&combo);
        set_kind(&mut st, K_KEY);
        fit.check(&st, &format!("profile {profile}, combo"));
        store.set_inspector_tab(1);
        fit.check(&st, &format!("profile {profile}, all bindings"));
        store.set_inspector_tab(0);
        st.select_input("mouse:dpi");
        fit.check(&st, &format!("profile {profile}, firmware button"));
    }

    // Macro editor: no step selected, then each kind of step, and recording.
    st.view = 1;
    st.macro_idx = Some(0);
    st.step = None;
    fit.check(&st, "macro, no step selected");
    for step in 0..6 {
        st.step = Some(step);
        fit.check(&st, &format!("macro, step {step} selected"));
    }
    st.recording = true;
    fit.check(&st, "macro, recording");
    st.recording = false;

    // Settings.
    st.view = 2;
    st.adding_device = true;
    fit.check(&st, "settings");

    assert!(fit.checked > 100, "only {} states checked", fit.checked);
    assert!(
        fit.failures.is_empty(),
        "{} problem(s) at the minimum window size ({MIN_W} x {MIN_H}):\n{}",
        fit.failures.len(),
        fit.failures.join("\n")
    );
}
