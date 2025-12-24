use std::{
    env,
    ffi::c_void,
    fs::OpenOptions,
    io::Write,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
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

use crate::tray::commands::{decode, Command};
use crate::tray::menu::{MenuItem, MenuSpec};
use crate::tray::model::{ModelUpdate, TrayModel};
use crate::tray::startup::StartupManager;

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

        let mut app_state = Box::new(MacApp::new()?);
        let app_ptr: *mut MacApp = &mut *app_state;

        let target = new_target(app_ptr);
        app_state
            .ui
            .install_status_item(target)
            .context("install status item")?;
        app_state.rebuild_menu().context("build menu")?;
        app_state.refresh_tooltip();

        let update = app_state
            .model
            .handle(Command::Reload, &app_state.startup)
            .context("initial reload")?;
        app_state.apply_update(update)?;

        app.run();
        drop(app_state);
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

struct MacTrayUi {
    status_item: Option<id>,
    menu: Option<id>,
    target: Option<id>,
}

impl MacTrayUi {
    fn install_status_item(&mut self, target: id) -> Result<()> {
        unsafe {
            let status_item: id =
                msg_send![NSStatusBar::systemStatusBar(nil), statusItemWithLength: -1.0];
            let button: id = msg_send![status_item, button];
            let title = nsstring(APP_NAME);
            let _: () = msg_send![button, setTitle: title];

            self.status_item = Some(status_item);
            self.target = Some(target);
            self.set_tooltip(APP_NAME);
        }

        Ok(())
    }

    fn rebuild_menu(&mut self, target: id, spec: &MenuSpec) -> Result<()> {
        unsafe {
            let menu: id = msg_send![class!(NSMenu), alloc];
            let menu: id = msg_send![menu, initWithTitle: nsstring(APP_NAME)];

            for item in &spec.items {
                match item {
                    MenuItem::Header(title) => add_header(menu, title),
                    MenuItem::Separator => {
                        let sep: id = msg_send![class!(NSMenuItem), separatorItem];
                        let _: () = msg_send![menu, addItem: sep];
                    }
                    MenuItem::Action {
                        id,
                        title,
                        checked,
                        enabled,
                    } => add_action_item(
                        menu,
                        title,
                        sel!(onMenuItem:),
                        target,
                        *id as NSInteger,
                        Some(*checked),
                        *enabled,
                    ),
                }
            }

            if let Some(status_item) = self.status_item {
                let _: () = msg_send![status_item, setMenu: menu];
            }

            self.menu = Some(menu);
        }
        Ok(())
    }

    fn set_tooltip(&mut self, text: &str) {
        unsafe {
            let status_item = match self.status_item {
                Some(s) => s,
                None => return,
            };
            let button: id = msg_send![status_item, button];
            let tip = nsstring(text);
            let _: () = msg_send![button, setToolTip: tip];
        }
    }
}

struct MacStartupManager;

impl StartupManager for MacStartupManager {
    fn is_enabled(&self) -> Result<bool> {
        autostart::is_enabled().context("read launch agent")
    }

    fn set_enabled(&self, enabled: bool) -> Result<()> {
        autostart::set_enabled(enabled).context("update launch agent")
    }
}

struct MacApp {
    ui: MacTrayUi,
    model: TrayModel,
    startup: MacStartupManager,
}

impl MacApp {
    fn new() -> Result<Self> {
        Ok(Self {
            ui: MacTrayUi {
                status_item: None,
                menu: None,
                target: None,
            },
            model: TrayModel::new()?,
            startup: MacStartupManager,
        })
    }

    fn rebuild_menu(&mut self) -> Result<()> {
        let spec = self.model.menu_spec();
        let Some(target) = self.ui.target else {
            return Err(anyhow!("menu target not set"));
        };
        self.ui.rebuild_menu(target, &spec)
    }

    fn refresh_tooltip(&mut self) {
        let tip = self.model.last_error().unwrap_or(APP_NAME);
        self.ui.set_tooltip(tip);
    }

    fn handle_menu_click(&mut self, cmd_id: u16) -> Result<()> {
        let Some(cmd) = decode(cmd_id, self.model.inputs()) else {
            return Ok(());
        };

        let update = self.model.handle(cmd, &self.startup)?;
        self.apply_update(update)
    }

    fn apply_update(&mut self, update: ModelUpdate) -> Result<()> {
        if let Some(path) = update.open_path {
            if let Err(err) = shell_open(&path) {
                let update = self.model.note_error(err);
                self.apply_update(update)?;
                return Ok(());
            }
        }

        if update.refresh_menu {
            self.rebuild_menu()?;
        }

        if update.refresh_tooltip {
            self.refresh_tooltip();
        }

        if update.quit {
            unsafe {
                let app = NSApp();
                let _: () = msg_send![app, terminate: nil];
            }
        }

        Ok(())
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
    enabled: bool,
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

    let _: () = msg_send![item, setEnabled: enabled];
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

fn new_target(state_ptr: *mut MacApp) -> id {
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
        let app = &mut *(state_ptr as *mut MacApp);
        let tag: NSInteger = msg_send![sender, tag];
        let cmd = tag as u16;

        if let Err(err) = app.handle_menu_click(cmd) {
            let msg = err.to_string();
            log_to_tmp("monitortray error", &msg);
            let update = app.model.note_error(err);
            let _ = app.apply_update(update);
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

fn shell_open(path: &Path) -> Result<()> {
    let status = ProcessCommand::new("open")
        .arg(path)
        .status()
        .with_context(|| format!("running open {}", path.display()))?;
    if !status.success() {
        return Err(anyhow!("open failed (exit={status})"));
    }
    Ok(())
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
        ProcessCommand::new("id")
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
        let out = ProcessCommand::new("launchctl")
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
