use std::{collections::BTreeMap, mem::size_of, path::Path};

use anyhow::{anyhow, Context, Result};
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
                CREATESTRUCTW, HMENU, ICONINFO, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR,
                MF_STRING, MF_UNCHECKED, MSG, SW_SHOWNORMAL, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
                TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_LBUTTONUP, WM_NCCREATE, WM_RBUTTONUP, WM_USER,
                WNDCLASSW, WS_OVERLAPPED,
            },
        },
    },
};

use crate::platform::Backend;
use crate::{config, platform, tray::common};

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
        let (start_with_windows, startup_error) = common::apply_startup_pref(
            start_with_windows_pref,
            |enabled| autostart::set_enabled(enabled),
            || autostart::is_enabled(),
        );

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
            let label = format!("{} ({value})", common::pretty_input_label(name));
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
        let (start_with_windows, startup_error) = common::apply_startup_pref(
            start_with_windows_pref,
            |enabled| autostart::set_enabled(enabled),
            || autostart::is_enabled(),
        );
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

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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

    let inputs = common::build_inputs(&resolved.inputs, CMD_BASE_INPUT);

    Ok((resolved.display_selector, inputs, start_with_windows_pref))
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
