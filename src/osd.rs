//! On-screen notifications ("Profile: Genshin Impact", "Turbo on").
//!
//! A small box near the bottom of the screen the focused window is on, shown
//! for a moment. It's a plain Win32 window on its own thread that never takes
//! focus and lets clicks through: taking focus would count as "this app is
//! focused", which pauses remapping. Games in exclusive fullscreen draw over
//! it; borderless / windowed games show it.

#[cfg(windows)]
pub use win::show;

#[cfg(not(windows))]
pub fn show(text: &str) {
    crate::log!("notice: {text}");
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Mutex, Once};

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
        GetDC, GetMonitorInfoW, InvalidateRect, MonitorFromWindow, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
        SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CALCRECT, DT_CENTER, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, FW_SEMIBOLD, HFONT, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW, KillTimer, PostMessageW,
        RegisterClassW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
        HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, MSG, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_APP,
        WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    const WM_SHOW: u32 = WM_APP + 10;
    const HIDE_TIMER: usize = 1;
    const SHOW_MS: u32 = 1400;

    static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);
    static TEXT: Mutex<String> = Mutex::new(String::new());
    static START: Once = Once::new();
    /// Font and scale for the monitor last shown on.
    static FONT: Mutex<(isize, u32)> = Mutex::new((0, 0));

    fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }

    /// Show `text` for a moment (any thread).
    pub fn show(text: &str) {
        if let Ok(mut t) = TEXT.lock() {
            *t = text.to_string();
        }
        START.call_once(|| {
            std::thread::Builder::new().name("osd".into()).spawn(osd_thread).ok();
        });
        let hwnd = HWND_VALUE.load(Ordering::SeqCst);
        if hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_SHOW, WPARAM(0), LPARAM(0));
            }
        }
        // Otherwise the thread shows the text once its window exists.
    }

    fn osd_thread() {
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).ok();
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.map(Into::into).unwrap_or_default(),
                lpszClassName: w!("mouse-kb-remap-osd"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                w!("mouse-kb-remap-osd"),
                w!("mouse-kb-remap notification"),
                WS_POPUP,
                0,
                0,
                10,
                10,
                None,
                None,
                instance.map(Into::into),
                None,
            ) else {
                crate::log!("osd: couldn't create the notification window");
                return;
            };
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 238, LWA_ALPHA);
            HWND_VALUE.store(hwnd.0 as isize, Ordering::SeqCst);
            let _ = PostMessageW(Some(hwnd), WM_SHOW, WPARAM(0), LPARAM(0));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// The font for a monitor's scale (cached).
    unsafe fn font_for(dpi: u32) -> HFONT {
        let mut f = FONT.lock().unwrap_or_else(|e| e.into_inner());
        if f.0 == 0 || f.1 != dpi {
            if f.0 != 0 {
                let _ = unsafe { DeleteObject(HGDIOBJ(f.0 as *mut _)) };
            }
            let font = unsafe {
                CreateFontW(
                    // 20 px at 100 % scaling.
                    -(20 * dpi as i32 / 96),
                    0,
                    0,
                    0,
                    FW_SEMIBOLD.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                )
            };
            *f = (font.0 as isize, dpi);
        }
        HFONT(f.0 as *mut _)
    }

    fn text_wide() -> Vec<u16> {
        TEXT.lock()
            .map(|t| t.clone())
            .unwrap_or_default()
            .encode_utf16()
            .collect()
    }

    unsafe fn layout_and_show(hwnd: HWND) {
        unsafe {
            // The monitor of the focused window (where the user is looking).
            let fg = GetForegroundWindow();
            let monitor = if fg.is_invalid() {
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY)
            } else {
                MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
            };
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
                return;
            }
            let (mut dpi, mut _y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
            let scale = |v: i32| v * dpi as i32 / 96;
            let font = font_for(dpi);

            let mut text = text_wide();
            let dc = GetDC(Some(hwnd));
            let old = SelectObject(dc, font.into());
            let mut r = RECT::default();
            DrawTextW(dc, &mut text, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old);
            ReleaseDC(Some(hwnd), dc);

            let width = (r.right - r.left) + scale(40);
            let height = (r.bottom - r.top) + scale(22);
            let work = mi.rcWork;
            let x = work.left + ((work.right - work.left) - width) / 2;
            let y = work.bottom - height - scale(72);
            let radius = scale(12);
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
            SetWindowRgn(hwnd, Some(region), false);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(hwnd), None, true);
            let _ = KillTimer(Some(hwnd), HIDE_TIMER);
            SetTimer(Some(hwnd), HIDE_TIMER, SHOW_MS, None);
        }
    }

    unsafe fn paint(hwnd: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
            // Theme: the app's dark surface with its accent as a side bar.
            let bg = CreateSolidBrush(rgb(0x1e, 0x1f, 0x22));
            FillRect(dc, &rc, bg);
            let _ = DeleteObject(bg.into());
            let accent = CreateSolidBrush(rgb(0x6d, 0x8d, 0xff));
            let bar = RECT {
                right: rc.left + (rc.bottom - rc.top) / 8,
                ..rc
            };
            FillRect(dc, &bar, accent);
            let _ = DeleteObject(accent.into());

            let dpi = FONT.lock().map(|f| f.1).unwrap_or(96).max(96);
            let old = SelectObject(dc, font_for(dpi).into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, rgb(0xf2, 0xf3, 0xf5));
            let mut text = text_wide();
            let mut tr = RECT {
                left: rc.left + bar.right - bar.left,
                ..rc
            };
            DrawTextW(
                dc,
                &mut text,
                &mut tr,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            let _ = EndPaint(hwnd, &ps);
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_SHOW => {
                    layout_and_show(hwnd);
                    LRESULT(0)
                }
                WM_TIMER if wparam.0 == HIDE_TIMER => {
                    let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    LRESULT(0)
                }
                WM_PAINT => {
                    paint(hwnd);
                    LRESULT(0)
                }
                WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
                WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }
}
