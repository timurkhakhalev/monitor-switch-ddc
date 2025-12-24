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
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Sel},
    sel, sel_impl,
};

use crate::platform::Backend;
use crate::{config, platform, tray::common};

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
            .open("/tmp/monitortray.out")
        {
            let _ = libc::dup2(f.as_raw_fd(), libc::STDOUT_FILENO);
        }

        if let Ok(f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/monitortray.err")
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
        let (start_at_login, startup_error) = common::apply_startup_pref(
            start_at_login_pref,
            |enabled| autostart::set_enabled(enabled),
            || autostart::is_enabled(),
        );

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
                let label = format!("{} ({value})", common::pretty_input_label(name));
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
        let (start_at_login, startup_error) = common::apply_startup_pref(
            start_at_login_pref,
            |enabled| autostart::set_enabled(enabled),
            || autostart::is_enabled(),
        );
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
        Err(e) => {
            return (
                "1".to_string(),
                common::default_inputs(CMD_BASE_INPUT),
                None,
                Some(e.to_string()),
            )
        }
    };
    let start_at_login_pref = cfg.as_ref().and_then(|c| c.start_with_windows);

    let (displays, load_error) = match backend.list_displays() {
        Ok(report) => (report.displays, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };

    let resolved = config::resolve(cfg.as_ref(), &displays, None);
    let inputs = common::build_inputs(&resolved.inputs, CMD_BASE_INPUT);

    (
        resolved.display_selector,
        inputs,
        start_at_login_pref,
        load_error,
    )
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
        .open("/tmp/monitortray.err")
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
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }

        let exe = std::env::current_exe().context("current_exe")?;
        let plist = launch_agent_plist(&exe, LABEL);
        fs::write(&path, plist.as_bytes()).with_context(|| format!("write {}", path.display()))?;

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
            r#"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
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
