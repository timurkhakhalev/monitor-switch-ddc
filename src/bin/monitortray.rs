#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use anyhow::Result;

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    windows_tray::run()
}

#[cfg(target_os = "macos")]
fn main() -> Result<()> {
    macos_tray::run()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() -> Result<()> {
    anyhow::bail!("monitortray is only supported on Windows and macOS");
}

#[cfg(target_os = "windows")]
mod windows_tray {
    use std::{collections::BTreeMap, mem::size_of, path::Path};

    use anyhow::{anyhow, Context, Result};
    use monitorctl::{config, platform, platform::Backend};
    use windows::{
        core::{w, Error as WinError, PCWSTR},
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
            Graphics::Gdi::{
                CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
                DIB_RGB_COLORS, HBITMAP,
            },
            System::LibraryLoader::GetModuleHandleW,
            System::Registry::{
                RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
                RRF_RT_REG_SZ,
            },
            UI::{
                Shell::{
                    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
                    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DefWindowProcW, DestroyMenu,
                    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage,
                    RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
                    CREATESTRUCTW, HMENU, ICONINFO, MF_CHECKED, MF_DISABLED, MF_GRAYED,
                    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SW_SHOWNORMAL, TPM_BOTTOMALIGN,
                    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_LBUTTONUP, WM_NCCREATE,
                    WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
                },
            },
        },
    };

    const WM_TRAYICON: u32 = WM_USER + 1;

    const CMD_BASE_INPUT: u16 = 2000;
    const CMD_RELOAD: u16 = 5000;
    const CMD_QUIT: u16 = 5001;
    const CMD_TOGGLE_STARTUP: u16 = 5002;
    const CMD_EDIT_CONFIG: u16 = 5003;
    const CMD_OPEN_CONFIG_FOLDER: u16 = 5004;

    pub fn run() -> Result<()> {
        unsafe {
            let hinstance = HINSTANCE(GetModuleHandleW(None).context("GetModuleHandleW")?.0);

            let class_name = w!("monitortray.hidden-window");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            let atom = RegisterClassW(&wc);
            if atom == 0 {
                return Err(anyhow!("RegisterClassW failed"));
            }

            let mut state = Box::new(State::new()?);

            let hwnd = windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                class_name,
                w!("monitortray"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance),
                Some(state.as_mut() as *mut _ as *const _),
            )
            .context("CreateWindowExW")?;

            state.hwnd = Some(hwnd);
            state.install_tray_icon().context("install tray icon")?;
            // State is now owned by the window (freed on quit).
            let _ = Box::into_raw(state);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    struct State {
        hwnd: Option<HWND>,
        tray: NOTIFYICONDATAW,
        menu: Option<HMENU>,
        // Command id -> raw VCP value.
        inputs: BTreeMap<u16, (String, u16)>,
        display_selector: String,
        start_with_windows: bool,
        backend: Box<dyn Backend>,
        last_error: Option<String>,
    }

    impl State {
        fn new() -> Result<Self> {
            let backend = platform::backend().context("select backend")?;
            let (display_selector, inputs, start_with_windows_pref) =
                load_display_and_inputs(&*backend)?;
            let (start_with_windows, startup_error) = apply_startup_pref(start_with_windows_pref);

            Ok(Self {
                hwnd: None,
                tray: NOTIFYICONDATAW::default(),
                menu: None,
                inputs,
                display_selector,
                start_with_windows,
                backend,
                last_error: startup_error,
            })
        }

        fn hwnd(&self) -> Result<HWND> {
            self.hwnd
                .ok_or_else(|| anyhow!("internal error: hwnd not set yet"))
        }

        fn rebuild_menu(&mut self) -> Result<()> {
            unsafe {
                if let Some(menu) = self.menu.take() {
                    let _ = DestroyMenu(menu);
                }
            }

            let menu = unsafe { CreatePopupMenu() }.context("CreatePopupMenu")?;

            // Section header
            unsafe {
                AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, w!("Inputs"))
                    .context("AppendMenuW(header:inputs)")?;
            }

            for (cmd, (name, value)) in &self.inputs {
                let label = format!("{} ({value})", pretty_input_label(name));
                let wlabel = wide(&label);
                unsafe {
                    AppendMenuW(
                        menu,
                        MF_STRING,
                        *cmd as usize,
                        PCWSTR::from_raw(wlabel.as_ptr()),
                    )
                }
                .context("AppendMenuW(input)")?;
            }

            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }
                .context("AppendMenuW(separator)")?;

            unsafe {
                AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 0, w!("Actions"))
                    .context("AppendMenuW(header:actions)")?;
            }
            unsafe {
                let flags = MF_STRING
                    | if self.start_with_windows {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                AppendMenuW(
                    menu,
                    flags,
                    CMD_TOGGLE_STARTUP as usize,
                    w!("Start with Windows"),
                )
                .context("AppendMenuW(startup)")?;
                AppendMenuW(menu, MF_STRING, CMD_EDIT_CONFIG as usize, w!("Edit config"))
                    .context("AppendMenuW(edit config)")?;
                AppendMenuW(
                    menu,
                    MF_STRING,
                    CMD_OPEN_CONFIG_FOLDER as usize,
                    w!("Open config folder"),
                )
                .context("AppendMenuW(open config folder)")?;
                AppendMenuW(menu, MF_STRING, CMD_RELOAD as usize, w!("Reload config"))
                    .context("AppendMenuW(reload)")?;
                AppendMenuW(menu, MF_STRING, CMD_QUIT as usize, w!("Quit"))
                    .context("AppendMenuW(quit)")?;
            }

            self.menu = Some(menu);
            Ok(())
        }

        fn install_tray_icon(&mut self) -> Result<()> {
            let hwnd = self.hwnd()?;
            self.rebuild_menu().context("build menu")?;

            let icon = create_tray_icon().unwrap_or_else(|_| {
                unsafe {
                    LoadIconW(
                        None,
                        windows::Win32::UI::WindowsAndMessaging::IDI_APPLICATION,
                    )
                }
                .unwrap_or_default()
            });
            let tip = "monitortray";

            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_ICON;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = icon;
            write_tip(&mut nid, tip);

            shell_notify_icon(NIM_ADD, &nid).context("Shell_NotifyIconW(NIM_ADD)")?;
            self.tray = nid;

            Ok(())
        }

        fn update_tooltip(&mut self) {
            let tip = match self.last_error.as_deref() {
                None => "monitortray",
                Some(e) => e,
            };
            write_tip(&mut self.tray, tip);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &self.tray);
            }
        }

        fn show_menu_and_handle(&mut self) -> Result<()> {
            let hwnd = self.hwnd()?;
            let Some(menu) = self.menu else {
                return Ok(());
            };

            unsafe {
                let mut pt = POINT::default();
                GetCursorPos(&mut pt).context("GetCursorPos")?;
                let _ = SetForegroundWindow(hwnd);
                let cmd = TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
                    pt.x,
                    pt.y,
                    None,
                    hwnd,
                    None,
                );

                if cmd.0 == 0 {
                    return Ok(());
                }

                let cmd = cmd.0 as u16;
                match cmd {
                    CMD_QUIT => {
                        self.remove_tray_icon();
                        PostQuitMessage(0);
                    }
                    CMD_RELOAD => {
                        self.reload_config().context("reload config")?;
                    }
                    CMD_TOGGLE_STARTUP => {
                        self.toggle_start_with_windows().context("toggle startup")?;
                    }
                    CMD_EDIT_CONFIG => {
                        self.edit_config().context("edit config")?;
                    }
                    CMD_OPEN_CONFIG_FOLDER => {
                        self.open_config_folder().context("open config folder")?;
                    }
                    _ => {
                        if let Some((_name, value)) = self.inputs.get(&cmd).cloned() {
                            self.set_input(value).context("set input")?;
                        }
                    }
                }
            }

            Ok(())
        }

        fn set_input(&mut self, value: u16) -> Result<()> {
            self.backend
                .set_input(&self.display_selector, value)
                .with_context(|| format!("set input {value} on '{}'", self.display_selector))?;
            self.last_error = None;
            self.update_tooltip();
            Ok(())
        }

        fn reload_config(&mut self) -> Result<()> {
            let (display_selector, inputs, start_with_windows_pref) =
                load_display_and_inputs(&*self.backend)?;
            self.display_selector = display_selector;
            self.inputs = inputs;
            let (start_with_windows, startup_error) = apply_startup_pref(start_with_windows_pref);
            self.start_with_windows = start_with_windows;
            self.rebuild_menu()?;
            self.last_error = startup_error;
            self.update_tooltip();
            Ok(())
        }

        fn toggle_start_with_windows(&mut self) -> Result<()> {
            let next = !self.start_with_windows;
            autostart::set_enabled(next).context("update registry startup entry")?;
            let _path = config::patch_start_with_windows(next).context("update config")?;
            self.start_with_windows = next;
            self.rebuild_menu()?;
            self.last_error = None;
            self.update_tooltip();
            Ok(())
        }

        fn edit_config(&mut self) -> Result<()> {
            let path = config::ensure_config_file_exists().context("ensure config exists")?;
            shell_open(&path).with_context(|| format!("open {}", path.display()))?;
            Ok(())
        }

        fn open_config_folder(&mut self) -> Result<()> {
            let Some(path) = config::resolve_config_path() else {
                return Err(anyhow!("No config path available"));
            };
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("No parent directory for config path"))?;
            shell_open(parent).with_context(|| format!("open {}", parent.display()))?;
            Ok(())
        }

        fn remove_tray_icon(&mut self) {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.tray);
                if let Some(menu) = self.menu.take() {
                    let _ = DestroyMenu(menu);
                }
            }
        }
    }

    impl Drop for State {
        fn drop(&mut self) {
            self.remove_tray_icon();
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let state_ptr = cs.lpCreateParams as *mut State;
                windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                    hwnd,
                    windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                    state_ptr as isize,
                );
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            WM_TRAYICON => {
                let state = get_state(hwnd);
                if state.is_null() {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let state = &mut *state;

                let evt = lparam.0 as u32;
                if evt == WM_RBUTTONUP || evt == WM_LBUTTONUP {
                    if let Err(e) = state.show_menu_and_handle() {
                        state.last_error = Some(e.to_string());
                        state.update_tooltip();
                    }
                    return LRESULT(0);
                }
            }
            windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY => {
                let state = get_state(hwnd);
                if !state.is_null() {
                    windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                        0,
                    );
                    drop(Box::from_raw(state));
                }
            }
            _ => {}
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn get_state(hwnd: HWND) -> *mut State {
        unsafe {
            let ptr = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            );
            ptr as *mut State
        }
    }

    fn load_display_and_inputs(
        backend: &dyn Backend,
    ) -> Result<(String, BTreeMap<u16, (String, u16)>, Option<bool>)> {
        let report = backend.list_displays().context("list displays")?;
        let cfg = config::load_optional()?;
        let resolved = config::resolve(cfg.as_ref(), &report.displays, None);
        let start_with_windows_pref = cfg.as_ref().and_then(|c| c.start_with_windows);

        let mut inputs: BTreeMap<u16, (String, u16)> = BTreeMap::new();
        let mut next_cmd = CMD_BASE_INPUT;

        if resolved.inputs.is_empty() {
            // Defaults for your XG27ACS setup; override with config for other monitors.
            for (k, v) in [("dp1", 15u16), ("usb_c", 26u16)] {
                inputs.insert(next_cmd, (k.to_string(), v));
                next_cmd += 1;
            }
        } else {
            let mut keys = resolved
                .inputs
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<Vec<_>>();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in keys {
                inputs.insert(next_cmd, (k, v));
                next_cmd += 1;
            }
        }

        Ok((resolved.display_selector, inputs, start_with_windows_pref))
    }

    fn apply_startup_pref(pref: Option<bool>) -> (bool, Option<String>) {
        match pref {
            Some(enabled) => match autostart::set_enabled(enabled) {
                Ok(()) => (enabled, None),
                Err(e) => (enabled, Some(e.to_string())),
            },
            None => match autostart::is_enabled() {
                Ok(enabled) => (enabled, None),
                Err(e) => (false, Some(e.to_string())),
            },
        }
    }

    fn shell_open(path: &Path) -> Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("Non-UTF-8 path: {}", path.display()))?;
        let wpath = wide(path);
        unsafe {
            let h = ShellExecuteW(
                None,
                w!("open"),
                PCWSTR::from_raw(wpath.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            // Per Win32 docs: values <= 32 indicate an error.
            if (h.0 as isize) <= 32 {
                return Err(anyhow!("ShellExecuteW failed ({})", h.0 as isize));
            }
        }
        Ok(())
    }

    mod autostart {
        use super::*;
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, WIN32_ERROR};
        use windows::Win32::System::Registry::REG_VALUE_TYPE;

        const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        const VALUE_NAME: &str = "monitortray";
        const OK: WIN32_ERROR = WIN32_ERROR(0);

        pub fn is_enabled() -> Result<bool> {
            Ok(read_value()?.is_some())
        }

        pub fn set_enabled(enabled: bool) -> Result<()> {
            if enabled {
                let cmd = autostart_command()?;
                write_value(&cmd)?;
            } else {
                delete_value()?;
            }
            Ok(())
        }

        fn autostart_command() -> Result<String> {
            let exe = std::env::current_exe().context("current_exe")?;
            Ok(format!("\"{}\"", exe.display()))
        }

        fn read_value() -> Result<Option<String>> {
            let value_name = wide(VALUE_NAME);
            let mut typ = REG_VALUE_TYPE::default();
            let mut bytes: u32 = 0;
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    PCWSTR::from_raw(value_name.as_ptr()),
                    RRF_RT_REG_SZ,
                    Some(&mut typ as *mut REG_VALUE_TYPE),
                    None,
                    Some(&mut bytes),
                )
            };

            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if status != OK {
                return Err(anyhow!("RegGetValueW(size) failed: {status:?}"));
            }

            let mut buf: Vec<u16> = vec![0u16; (bytes as usize / 2).max(1)];
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    PCWSTR::from_raw(value_name.as_ptr()),
                    RRF_RT_REG_SZ,
                    Some(&mut typ as *mut REG_VALUE_TYPE),
                    Some(buf.as_mut_ptr() as *mut _),
                    Some(&mut bytes),
                )
            };
            if status != OK {
                return Err(anyhow!("RegGetValueW(data) failed: {status:?}"));
            }

            let len = (bytes as usize / 2).saturating_sub(1);
            Ok(Some(String::from_utf16_lossy(&buf[..len])))
        }

        fn write_value(cmd: &str) -> Result<()> {
            let value_name = wide(VALUE_NAME);
            let cmd = wide(cmd);
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    PCWSTR::from_raw(value_name.as_ptr()),
                    REG_SZ.0,
                    Some(cmd.as_ptr() as *const _),
                    (cmd.len() * 2) as u32,
                )
            };
            if status != OK {
                return Err(anyhow!("RegSetKeyValueW failed: {status:?}"));
            }
            Ok(())
        }

        fn delete_value() -> Result<()> {
            let value_name = wide(VALUE_NAME);
            let status = unsafe {
                RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    PCWSTR::from_raw(value_name.as_ptr()),
                )
            };
            if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
                return Ok(());
            }
            if status != OK {
                return Err(anyhow!("RegDeleteKeyValueW failed: {status:?}"));
            }
            Ok(())
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    fn write_tip(nid: &mut NOTIFYICONDATAW, tip: &str) {
        let tip = tip.chars().take(127).collect::<String>();
        let mut buf = [0u16; 128];
        for (i, c) in tip.encode_utf16().take(127).enumerate() {
            buf[i] = c;
        }
        nid.szTip = buf;
    }

    fn shell_notify_icon(action: NOTIFY_ICON_MESSAGE, nid: &NOTIFYICONDATAW) -> Result<()> {
        let ok = unsafe { Shell_NotifyIconW(action, nid) };
        if !ok.as_bool() {
            return Err(anyhow!(WinError::from_thread()));
        }
        Ok(())
    }

    fn pretty_input_label(key: &str) -> &str {
        match key {
            "dp1" => "DisplayPort 1",
            "dp2" => "DisplayPort 2",
            "usb_c" => "USB-C",
            "usbc" => "USB-C",
            "hdmi1" => "HDMI 1",
            "hdmi2" => "HDMI 2",
            _ => key,
        }
    }

    fn create_tray_icon() -> Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
        // Create a simple 32x32 ARGB icon (dark background + blue "monitor" outline).
        const W: i32 = 32;
        const H: i32 = 32;

        unsafe {
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: W,
                biHeight: -H, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            };

            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let color: HBITMAP = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .context("CreateDIBSection")?;

            if bits.is_null() {
                return Err(anyhow!("CreateDIBSection returned null bits"));
            }

            let pixels = bits as *mut u32;
            let bg: u32 = 0xFF1B1E24; // AARRGGBB
            let stroke: u32 = 0xFF2A9DF4;
            let stroke2: u32 = 0xFF66D9EF;

            for y in 0..H {
                for x in 0..W {
                    *pixels.offset((y * W + x) as isize) = bg;
                }
            }

            // Monitor rectangle border
            for x in 6..26 {
                *pixels.offset((6 * W + x) as isize) = stroke;
                *pixels.offset((22 * W + x) as isize) = stroke;
            }
            for y in 6..23 {
                *pixels.offset((y * W + 6) as isize) = stroke;
                *pixels.offset((y * W + 25) as isize) = stroke;
            }
            // Inner accent
            for x in 8..24 {
                *pixels.offset((8 * W + x) as isize) = stroke2;
            }
            // Stand
            for y in 23..28 {
                *pixels.offset((y * W + 15) as isize) = stroke;
                *pixels.offset((y * W + 16) as isize) = stroke;
            }
            for x in 11..21 {
                *pixels.offset((28 * W + x) as isize) = stroke;
            }

            // Mask bitmap (all-zeros = opaque everywhere).
            let mask = CreateBitmap(W, H, 1, 1, None);
            if mask.0.is_null() {
                let _ = DeleteObject(color.into());
                return Err(anyhow!(WinError::from_thread())).context("CreateBitmap(mask)");
            }

            let icon_info = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            };

            let icon = CreateIconIndirect(&icon_info).context("CreateIconIndirect")?;

            let _ = DeleteObject(mask.into());
            let _ = DeleteObject(color.into());

            Ok(icon)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_tray {
    use std::{
        collections::BTreeMap,
        env,
        ffi::c_void,
        fs::OpenOptions,
        io::Write,
        os::fd::AsRawFd,
        path::{Path, PathBuf},
        process::Command,
        sync::Once,
    };

    use anyhow::{anyhow, Context, Result};
    use cocoa::{
        appkit::{NSApp, NSApplication, NSApplicationActivationPolicyAccessory, NSStatusBar},
        base::{id, nil},
        foundation::{NSAutoreleasePool, NSInteger, NSString},
    };
    use monitorctl::{config, platform, platform::Backend};
    use objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    };

    const CMD_BASE_INPUT: u16 = 2000;
    const CMD_RELOAD: u16 = 5000;
    const CMD_QUIT: u16 = 5001;
    const CMD_TOGGLE_STARTUP: u16 = 5002;
    const CMD_EDIT_CONFIG: u16 = 5003;
    const CMD_OPEN_CONFIG_FOLDER: u16 = 5004;

    const APP_NAME: &str = "monitorctl";

    const MENU_STATE_OFF: NSInteger = 0;
    const MENU_STATE_ON: NSInteger = 1;

    pub fn run() -> Result<()> {
        unsafe {
            // If the user launches the binary from a terminal (e.g. `cargo run --bin monitortray`),
            // closing that terminal will typically send SIGHUP and kill the app. Ignore SIGHUP and
            // redirect stdout/stderr to avoid being tied to the tty.
            detach_from_terminal();

            let _pool = NSAutoreleasePool::new(nil);

            let app = NSApp();
            app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);

            let mut state = Box::new(State::new()?);
            let state_ptr: *mut State = &mut *state;

            let target = new_target(state_ptr);
            state
                .install_status_item(target)
                .context("install status item")?;

            app.run();
            drop(state);
        }

        Ok(())
    }

    fn detach_from_terminal() {
        unsafe {
            // Ignore SIGHUP (terminal close).
            libc::signal(libc::SIGHUP, libc::SIG_IGN);

            // If we're attached to a tty, redirect stdout/stderr to files.
            // This keeps the process alive even if the tty goes away.
            let stdout_is_tty = libc::isatty(libc::STDOUT_FILENO) == 1;
            let stderr_is_tty = libc::isatty(libc::STDERR_FILENO) == 1;
            if !stdout_is_tty && !stderr_is_tty {
                return;
            }

            if let Ok(f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/monitorctl.out")
            {
                let _ = libc::dup2(f.as_raw_fd(), libc::STDOUT_FILENO);
            }

            if let Ok(f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/monitorctl.err")
            {
                let _ = libc::dup2(f.as_raw_fd(), libc::STDERR_FILENO);
            }
        }
    }

    struct State {
        status_item: Option<id>,
        menu: Option<id>,
        // Command id -> (name, raw VCP value).
        inputs: BTreeMap<u16, (String, u16)>,
        display_selector: String,
        start_at_login: bool,
        backend: Box<dyn Backend>,
        last_error: Option<String>,
    }

    impl State {
        fn new() -> Result<Self> {
            let backend = platform::backend().context("select backend")?;
            let (display_selector, inputs, start_at_login_pref, load_error) =
                load_display_and_inputs(&*backend);
            let (start_at_login, startup_error) = apply_startup_pref(start_at_login_pref);

            let last_error = load_error.or(startup_error);

            Ok(Self {
                status_item: None,
                menu: None,
                inputs,
                display_selector,
                start_at_login,
                backend,
                last_error,
            })
        }

        fn install_status_item(&mut self, target: id) -> Result<()> {
            unsafe {
                let status_item: id =
                    msg_send![NSStatusBar::systemStatusBar(nil), statusItemWithLength: -1.0];
                let button: id = msg_send![status_item, button];
                let title = nsstring(APP_NAME);
                let _: () = msg_send![button, setTitle: title];

                self.status_item = Some(status_item);
                self.rebuild_menu(target).context("build menu")?;
                self.update_tooltip();
            }

            Ok(())
        }

        fn rebuild_menu(&mut self, target: id) -> Result<()> {
            unsafe {
                let menu: id = msg_send![class!(NSMenu), alloc];
                let menu: id = msg_send![menu, initWithTitle: nsstring(APP_NAME)];

                // Section header (disabled).
                add_header(menu, "Inputs");

                for (cmd, (name, value)) in &self.inputs {
                    let label = format!("{} ({value})", pretty_input_label(name));
                    add_action_item(
                        menu,
                        &label,
                        sel!(onMenuItem:),
                        target,
                        *cmd as NSInteger,
                        None,
                    );
                }

                let sep: id = msg_send![class!(NSMenuItem), separatorItem];
                let _: () = msg_send![menu, addItem: sep];

                add_header(menu, "Actions");

                add_action_item(
                    menu,
                    "Start at login",
                    sel!(onMenuItem:),
                    target,
                    CMD_TOGGLE_STARTUP as NSInteger,
                    Some(self.start_at_login),
                );
                add_action_item(
                    menu,
                    "Edit config",
                    sel!(onMenuItem:),
                    target,
                    CMD_EDIT_CONFIG as NSInteger,
                    None,
                );
                add_action_item(
                    menu,
                    "Open config folder",
                    sel!(onMenuItem:),
                    target,
                    CMD_OPEN_CONFIG_FOLDER as NSInteger,
                    None,
                );
                add_action_item(
                    menu,
                    "Reload config",
                    sel!(onMenuItem:),
                    target,
                    CMD_RELOAD as NSInteger,
                    None,
                );
                add_action_item(
                    menu,
                    "Quit",
                    sel!(onMenuItem:),
                    target,
                    CMD_QUIT as NSInteger,
                    None,
                );

                if let Some(status_item) = self.status_item {
                    let _: () = msg_send![status_item, setMenu: menu];
                }

                self.menu = Some(menu);
            }
            Ok(())
        }

        fn update_tooltip(&mut self) {
            unsafe {
                let status_item = match self.status_item {
                    Some(s) => s,
                    None => return,
                };
                let button: id = msg_send![status_item, button];
                let tip = match self.last_error.as_deref() {
                    None => APP_NAME,
                    Some(e) => e,
                };
                let tip = nsstring(tip);
                let _: () = msg_send![button, setToolTip: tip];
            }
        }

        fn handle_cmd(&mut self, cmd: u16, target: id) -> Result<()> {
            match cmd {
                CMD_QUIT => unsafe {
                    let app = NSApp();
                    let _: () = msg_send![app, terminate: nil];
                    Ok(())
                },
                CMD_RELOAD => {
                    self.reload_config(target).context("reload config")?;
                    Ok(())
                }
                CMD_TOGGLE_STARTUP => {
                    self.toggle_start_at_login(target)
                        .context("toggle startup")?;
                    Ok(())
                }
                CMD_EDIT_CONFIG => self.edit_config().context("edit config"),
                CMD_OPEN_CONFIG_FOLDER => self.open_config_folder().context("open config folder"),
                _ => {
                    if let Some((_name, value)) = self.inputs.get(&cmd).cloned() {
                        self.set_input(value).context("set input")?;
                    }
                    Ok(())
                }
            }
        }

        fn set_input(&mut self, value: u16) -> Result<()> {
            self.backend
                .set_input(&self.display_selector, value)
                .with_context(|| format!("set input {value} on '{}'", self.display_selector))?;
            self.last_error = None;
            self.update_tooltip();
            Ok(())
        }

        fn reload_config(&mut self, target: id) -> Result<()> {
            let (display_selector, inputs, start_at_login_pref, load_error) =
                load_display_and_inputs(&*self.backend);
            self.display_selector = display_selector;
            self.inputs = inputs;
            let (start_at_login, startup_error) = apply_startup_pref(start_at_login_pref);
            self.start_at_login = start_at_login;
            self.rebuild_menu(target)?;
            self.last_error = load_error.or(startup_error);
            self.update_tooltip();
            Ok(())
        }

        fn toggle_start_at_login(&mut self, target: id) -> Result<()> {
            let next = !self.start_at_login;
            autostart::set_enabled(next).context("update launch agent")?;
            let _path = config::patch_start_with_windows(next).context("update config")?;
            self.start_at_login = next;
            self.rebuild_menu(target)?;
            self.last_error = None;
            self.update_tooltip();
            Ok(())
        }

        fn edit_config(&mut self) -> Result<()> {
            let path = config::ensure_config_file_exists().context("ensure config exists")?;
            shell_open(&path).with_context(|| format!("open {}", path.display()))?;
            Ok(())
        }

        fn open_config_folder(&mut self) -> Result<()> {
            let Some(path) = config::resolve_config_path() else {
                return Err(anyhow!("No config path available"));
            };
            let parent = path
                .parent()
                .ok_or_else(|| anyhow!("No parent directory for config path"))?;
            shell_open(parent).with_context(|| format!("open {}", parent.display()))?;
            Ok(())
        }
    }

    fn load_display_and_inputs(
        backend: &dyn Backend,
    ) -> (
        String,
        BTreeMap<u16, (String, u16)>,
        Option<bool>,
        Option<String>,
    ) {
        let cfg = match config::load_optional() {
            Ok(v) => v,
            Err(e) => return ("1".to_string(), default_inputs(), None, Some(e.to_string())),
        };
        let start_at_login_pref = cfg.as_ref().and_then(|c| c.start_with_windows);

        let (displays, load_error) = match backend.list_displays() {
            Ok(report) => (report.displays, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };

        let resolved = config::resolve(cfg.as_ref(), &displays, None);
        let inputs = build_inputs(&resolved.inputs);

        (
            resolved.display_selector,
            inputs,
            start_at_login_pref,
            load_error,
        )
    }

    fn build_inputs(
        inputs: &std::collections::HashMap<String, u16>,
    ) -> BTreeMap<u16, (String, u16)> {
        if inputs.is_empty() {
            return default_inputs();
        }

        let mut keys = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<Vec<_>>();
        keys.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out: BTreeMap<u16, (String, u16)> = BTreeMap::new();
        let mut next_cmd = CMD_BASE_INPUT;
        for (k, v) in keys {
            out.insert(next_cmd, (k, v));
            next_cmd += 1;
        }
        out
    }

    fn default_inputs() -> BTreeMap<u16, (String, u16)> {
        // Defaults for your XG27ACS setup; override with config for other monitors.
        let mut inputs: BTreeMap<u16, (String, u16)> = BTreeMap::new();
        let mut next_cmd = CMD_BASE_INPUT;
        for (k, v) in [("dp1", 15u16), ("usb_c", 26u16)] {
            inputs.insert(next_cmd, (k.to_string(), v));
            next_cmd += 1;
        }
        inputs
    }

    fn apply_startup_pref(pref: Option<bool>) -> (bool, Option<String>) {
        match pref {
            Some(enabled) => match autostart::set_enabled(enabled) {
                Ok(()) => (enabled, None),
                Err(e) => (enabled, Some(e.to_string())),
            },
            None => match autostart::is_enabled() {
                Ok(enabled) => (enabled, None),
                Err(e) => (false, Some(e.to_string())),
            },
        }
    }

    fn shell_open(path: &Path) -> Result<()> {
        let status = Command::new("open")
            .arg(path)
            .status()
            .with_context(|| format!("running open {}", path.display()))?;
        if !status.success() {
            return Err(anyhow!("open failed (exit={status})"));
        }
        Ok(())
    }

    fn pretty_input_label(key: &str) -> &str {
        match key {
            "dp1" => "DisplayPort 1",
            "dp2" => "DisplayPort 2",
            "usb_c" => "USB-C",
            "usbc" => "USB-C",
            "hdmi1" => "HDMI 1",
            "hdmi2" => "HDMI 2",
            _ => key,
        }
    }

    unsafe fn nsstring(s: &str) -> id {
        NSString::alloc(nil).init_str(s)
    }

    unsafe fn add_header(menu: id, title: &str) {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let title = nsstring(title);
        let empty = nsstring("");
        let item: id =
            msg_send![item, initWithTitle: title action: sel!(onMenuItem:) keyEquivalent: empty];
        let _: () = msg_send![item, setEnabled: false];
        let _: () = msg_send![menu, addItem: item];
    }

    unsafe fn add_action_item(
        menu: id,
        title: &str,
        action: Sel,
        target: id,
        tag: NSInteger,
        checked: Option<bool>,
    ) {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let title = nsstring(title);
        let empty = nsstring("");
        let item: id = msg_send![item, initWithTitle: title action: action keyEquivalent: empty];
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![item, setTag: tag];

        if let Some(checked) = checked {
            let state = if checked {
                MENU_STATE_ON
            } else {
                MENU_STATE_OFF
            };
            let _: () = msg_send![item, setState: state];
        }

        let _: () = msg_send![menu, addItem: item];
    }

    fn target_class() -> *const Class {
        static ONCE: Once = Once::new();
        static mut CLS: *const Class = std::ptr::null();

        ONCE.call_once(|| unsafe {
            let ns_object = class!(NSObject);
            let mut decl = ClassDecl::new("MonitorTrayTarget", ns_object)
                .expect("MonitorTrayTarget class already registered");
            decl.add_ivar::<*mut c_void>("state_ptr");
            decl.add_method(
                sel!(onMenuItem:),
                on_menu_item as extern "C" fn(&Object, Sel, id),
            );
            CLS = decl.register();
        });

        unsafe { CLS }
    }

    fn new_target(state_ptr: *mut State) -> id {
        unsafe {
            let cls = target_class();
            let obj: id = msg_send![cls, new];
            (*obj).set_ivar("state_ptr", state_ptr as *mut c_void);
            obj
        }
    }

    extern "C" fn on_menu_item(this: &Object, _cmd: Sel, sender: id) {
        unsafe {
            let state_ptr: *mut c_void = *this.get_ivar("state_ptr");
            if state_ptr.is_null() {
                return;
            }
            let state = &mut *(state_ptr as *mut State);
            let tag: NSInteger = msg_send![sender, tag];
            let cmd = tag as u16;

            if let Err(e) = state.handle_cmd(cmd, this as *const _ as id) {
                let msg = e.to_string();
                log_to_tmp("monitortray error", &msg);
                state.last_error = Some(msg);
                state.update_tooltip();
            }
        }
    }

    fn log_to_tmp(prefix: &str, msg: &str) {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/monitorctl.err")
        {
            let _ = writeln!(f, "{prefix}: {msg}");
        }
    }

    mod autostart {
        use super::*;
        use std::fs;

        const LABEL: &str = "com.monitorctl.monitorctl";

        pub fn is_enabled() -> Result<bool> {
            Ok(plist_path()?.exists())
        }

        pub fn set_enabled(enabled: bool) -> Result<()> {
            if enabled {
                install()?;
            } else {
                uninstall()?;
            }
            Ok(())
        }

        fn install() -> Result<()> {
            let path = plist_path()?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }

            let exe = std::env::current_exe().context("current_exe")?;
            let plist = launch_agent_plist(&exe, LABEL);
            fs::write(&path, plist.as_bytes())
                .with_context(|| format!("write {}", path.display()))?;

            let uid = gui_uid();
            let domain = format!("gui/{uid}");

            // Best-effort: unload any previous instance, then load.
            let _ = launchctl(&["bootout", &domain, &path.to_string_lossy()]);
            launchctl(&["bootstrap", &domain, &path.to_string_lossy()])
                .context("launchctl bootstrap")?;
            let _ = launchctl(&["enable", &format!("{domain}/{LABEL}")]);

            Ok(())
        }

        fn uninstall() -> Result<()> {
            let path = plist_path()?;
            if !path.exists() {
                return Ok(());
            }

            let uid = gui_uid();
            let domain = format!("gui/{uid}");
            let _ = launchctl(&["disable", &format!("{domain}/{LABEL}")]);
            let _ = launchctl(&["bootout", &domain, &path.to_string_lossy()]);

            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            Ok(())
        }

        fn plist_path() -> Result<PathBuf> {
            let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
            Ok(PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LABEL}.plist")))
        }

        fn gui_uid() -> u32 {
            // LaunchAgents don't always inherit a useful env, so shell out to `id -u`.
            Command::new("id")
                .arg("-u")
                .output()
                .ok()
                .and_then(|out| {
                    if !out.status.success() {
                        return None;
                    }
                    String::from_utf8(out.stdout)
                        .ok()
                        .and_then(|s| s.trim().parse::<u32>().ok())
                })
                .unwrap_or(0)
        }

        fn launch_agent_plist(exe: &Path, label: &str) -> String {
            let exe = exe.display().to_string();
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>/tmp/monitortray.out</string>
  <key>StandardErrorPath</key><string>/tmp/monitortray.err</string>
</dict>
</plist>
"#
            )
        }

        fn launchctl(args: &[&str]) -> Result<()> {
            let out = Command::new("launchctl")
                .args(args)
                .output()
                .with_context(|| format!("running launchctl {}", args.join(" ")))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                return Err(anyhow!(
                    "launchctl failed (exit={}):\nstdout:\n{}\nstderr:\n{}",
                    out.status,
                    stdout.trim(),
                    stderr.trim()
                ));
            }
            Ok(())
        }
    }
}
