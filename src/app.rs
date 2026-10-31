//! The settings window (Slint, ui/app.slint) and tray icon.
//!
//! Closing the window hides it to the tray; Quit is in the tray menu. While
//! hidden nothing runs on the UI thread: the meter timer only ticks when the
//! window is visible, and status changes arrive through invoke_from_event_loop.
//!
//! The App lives in a thread-local on the UI thread, so callbacks from the
//! window, tray and other threads reach it with `with_app` instead of holding
//! references to it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::bridge::{self, Bridge, Kind, Sink};
use crate::keys::{self, Hotkey};
use crate::settings::{data_dir, Mode, Settings};
use crate::{icons, util, APP_NAME};

slint::include_modules!();

const MAX_LOG_LINES: usize = 500;
const MAX_LOG_FILE: u64 = 1 << 20;

#[derive(Default)]
struct State {
    settings: Settings,
    bridge: Option<Bridge>,
    /// The user wants it listening (it may be waiting for the mic).
    want: bool,
    /// Starting failed and the retry timer is running.
    retrying: bool,
    /// What the status text/tray currently show, to skip no-op updates.
    shown: String,
}

pub struct App {
    ui: AppWindow,
    tray: TrayIcon,
    tray_icons: icons::Icons,
    mi_open: MenuItem,
    mi_listen: CheckMenuItem,
    mi_quit: MenuItem,

    meter_timer: Timer,
    retry_timer: Timer,

    state: RefCell<State>,
    logf: RefCell<Option<File>>,
    log: Rc<VecModel<LogLine>>,
    devices: RefCell<Vec<String>>,
    events: Arc<Mutex<VecDeque<(Kind, String)>>>,
    recording: Cell<bool>,
    hotkey_before: RefCell<String>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Run `f` with the app, if it's been built (UI thread only).
fn with_app(f: impl FnOnce(&App)) {
    let app = APP.with(|a| a.borrow().clone());
    if let Some(app) = app {
        f(&app);
    }
}

/// Show the window; used when a second copy of the app is started.
pub fn show_window() {
    with_app(|app| app.show());
}

fn kind_code(kind: Kind) -> i32 {
    match kind {
        Kind::Info => 0,
        Kind::Down => 1,
        Kind::Up => 2,
        Kind::Ignored => 3,
        Kind::Extra => 4,
        Kind::Error | Kind::StreamError => 5,
    }
}

impl App {
    pub fn build(tray_only: bool) -> Result<Rc<App>, String> {
        let settings = Settings::load();
        let ui = AppWindow::new().map_err(|e| e.to_string())?;
        let modes: Vec<SharedString> = Mode::ALL.iter().map(|m| m.label().into()).collect();
        ui.set_modes(ModelRc::new(VecModel::from(modes)));
        let log = Rc::new(VecModel::<LogLine>::default());
        ui.set_log(ModelRc::from(log.clone()));

        let tray_icons = icons::load();
        let mi_open = MenuItem::new("Open Handy Shortcut", true, None);
        let mi_listen = CheckMenuItem::new("Listening", true, false, None);
        let mi_quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&mi_open, &mi_listen, &PredefinedMenuItem::separator(), &mi_quit])
            .map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_icon(tray_icons.stopped.clone())
            .with_tooltip(APP_NAME)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        let app = Rc::new(App {
            ui,
            tray,
            tray_icons,
            mi_open,
            mi_listen,
            mi_quit,
            meter_timer: Timer::default(),
            retry_timer: Timer::default(),
            state: RefCell::new(State { settings: settings.clone(), ..State::default() }),
            logf: RefCell::new(open_log_file()),
            log,
            devices: RefCell::new(Vec::new()),
            events: Arc::new(Mutex::new(VecDeque::new())),
            recording: Cell::new(false),
            hotkey_before: RefCell::new(String::new()),
        });
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));

        app.load_form(&settings);
        app.bind_events();
        util::refresh_startup_path();
        app.refresh_devices();
        app.log(Kind::Info, &format!("{APP_NAME} {} started", env!("CARGO_PKG_VERSION")));
        if settings.autostart {
            app.state.borrow_mut().want = true;
            app.try_start();
        }
        app.refresh_status();
        app.update_save_btn();
        if !tray_only {
            app.show();
        }
        Ok(app)
    }

    fn load_form(&self, s: &Settings) {
        let ui = &self.ui;
        ui.set_hotkey(s.hotkey.as_str().into());
        ui.set_mode_index(Mode::ALL.iter().position(|&m| m == s.mode).unwrap_or(0) as i32);
        ui.set_enter_after(s.enter_after);
        ui.set_enter_delay(s.enter_delay_ms.min(i32::MAX as u32) as i32);
        ui.set_channel(s.channel.max(1) as i32);
        ui.set_invert(s.invert);
        ui.set_autostart(s.autostart);
        ui.set_startup(util::starts_with_windows());
        ui.set_peak_min(s.peak_min.to_string().into());
        ui.set_width_ms(s.width_ms.to_string().into());
    }

    fn bind_events(&self) {
        let ui = &self.ui;
        ui.on_toggle_listening(|| with_app(|a| a.toggle()));
        ui.on_record(|| with_app(|a| a.record_clicked()));
        ui.on_refresh_devices(|| with_app(|a| a.refresh_devices()));
        ui.on_save(|| with_app(|a| a.save_clicked()));
        ui.on_form_edited(|| with_app(|a| a.update_save_btn()));
        ui.on_apply_now(|| with_app(|a| a.apply_form()));
        ui.on_startup_toggled(|on| with_app(|a| a.toggle_startup(on)));
        ui.on_open_log(|| with_app(|a| a.open_log()));

        ui.window().on_close_requested(|| {
            with_app(|a| a.hide());
            slint::CloseRequestResponse::HideWindow
        });

        // Tray and menu events come from the tray's own window procedure;
        // hop onto the Slint event loop before touching the app.
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = slint::invoke_from_event_loop(show_window);
            }
        }));
        MenuEvent::set_event_handler(Some(|e: MenuEvent| {
            let id = e.id().clone();
            let _ = slint::invoke_from_event_loop(move || with_app(|a| a.menu_clicked(&id)));
        }));
    }

    fn menu_clicked(&self, id: &MenuId) {
        if id == self.mi_open.id() {
            self.show();
        } else if id == self.mi_listen.id() {
            self.toggle();
        } else if id == self.mi_quit.id() {
            self.quit();
        }
    }

    // ---- listening ----------------------------------------------------------

    fn sink(&self) -> Sink {
        let queue = self.events.clone();
        Arc::new(move |kind, text| {
            queue.lock().unwrap_or_else(|e| e.into_inner()).push_back((kind, text));
            let _ = slint::invoke_from_event_loop(|| with_app(|a| a.drain_events()));
        })
    }

    fn toggle(&self) {
        if self.state.borrow().want {
            self.stop();
        } else {
            self.start();
        }
    }

    fn start(&self) {
        let s = match self.collect_form() {
            Ok(s) => s,
            Err(e) => {
                self.ui.set_form_error(e.into());
                self.show();
                return;
            }
        };
        self.save(&s);
        {
            let mut st = self.state.borrow_mut();
            st.settings = s;
            st.want = true;
        }
        self.try_start();
        self.refresh_status();
        self.update_save_btn();
    }

    fn try_start(&self) {
        let s = self.state.borrow().settings.clone();
        match Bridge::start(&s, self.sink()) {
            Ok(b) => {
                self.retry_timer.stop();
                let mut st = self.state.borrow_mut();
                st.bridge = Some(b);
                st.retrying = false;
            }
            Err(e) => {
                let first = !self.state.borrow().retrying;
                self.state.borrow_mut().retrying = true;
                if first {
                    self.log(Kind::Error, &format!("Couldn't start: {e}. Retrying every 3 seconds..."));
                }
                if !self.retry_timer.running() {
                    self.retry_timer.start(TimerMode::Repeated, Duration::from_secs(3), || {
                        with_app(|a| a.retry())
                    });
                }
            }
        }
    }

    fn retry(&self) {
        let (want, running) = {
            let st = self.state.borrow();
            (st.want, st.bridge.is_some())
        };
        if !want || running {
            self.retry_timer.stop();
            return;
        }
        self.try_start();
        self.refresh_status();
    }

    fn stop(&self) {
        {
            let mut st = self.state.borrow_mut();
            st.want = false;
            st.retrying = false;
        }
        self.retry_timer.stop();
        self.stop_bridge();
        self.log(Kind::Info, "Stopped");
        self.refresh_status();
    }

    fn stop_bridge(&self) {
        let bridge = self.state.borrow_mut().bridge.take();
        drop(bridge); // releases the shortcut if held
    }

    fn drain_events(&self) {
        loop {
            let next = self.events.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some((kind, text)) = next else { break };
            if kind == Kind::StreamError {
                if self.state.borrow().bridge.is_none() {
                    continue; // already handled this break
                }
                self.log(Kind::Error, &format!("Microphone stopped working: {text}"));
                self.stop_bridge();
                if self.state.borrow().want {
                    self.state.borrow_mut().retrying = false;
                    self.try_start();
                }
            } else {
                self.log(kind, &text);
            }
        }
        self.refresh_status();
    }

    // ---- form ---------------------------------------------------------------

    /// Settings from the form, or why they don't parse.
    fn collect_form(&self) -> Result<Settings, String> {
        let ui = &self.ui;
        let cur = self.state.borrow().settings.clone();
        let (Ok(peak_min), Ok(width_ms)) = (
            ui.get_peak_min().trim().parse::<f32>(),
            ui.get_width_ms().trim().parse::<f32>(),
        ) else {
            return Err("Min peak and min width must be numbers.".into());
        };
        let hotkey = if self.recording.get() {
            cur.hotkey.clone()
        } else {
            ui.get_hotkey().trim().to_lowercase()
        };
        if let Err(e) = Hotkey::parse(&hotkey) {
            return Err(format!("'{hotkey}' isn't a shortcut I can press: {e}. Try ctrl+space."));
        }
        let device = usize::try_from(ui.get_device_index())
            .ok()
            .and_then(|i| self.devices.borrow().get(i).cloned())
            .unwrap_or_else(|| cur.device.clone());
        let mode = usize::try_from(ui.get_mode_index())
            .ok()
            .and_then(|i| Mode::ALL.get(i).copied())
            .unwrap_or(cur.mode);
        Ok(Settings {
            device,
            channel: ui.get_channel().max(1) as u32,
            hotkey,
            mode,
            invert: ui.get_invert(),
            enter_after: ui.get_enter_after(),
            enter_delay_ms: ui.get_enter_delay().max(0) as u32,
            autostart: ui.get_autostart(),
            peak_min,
            width_ms,
            ..cur
        })
    }

    fn save(&self, s: &Settings) {
        if let Err(e) = s.save() {
            self.log(Kind::Error, &format!("Couldn't save settings: {e}"));
        }
    }

    /// Save the form if it's valid and, if listening, restart with it. Used
    /// for switches and dropdowns, which apply right away.
    fn apply_form(&self) {
        if let Ok(s) = self.collect_form() {
            self.apply(s);
        }
        self.update_save_btn();
    }

    /// The Save button, or Enter in a text field.
    fn save_clicked(&self) {
        if self.recording.get() {
            return;
        }
        match self.collect_form() {
            Ok(s) => {
                if self.apply(s) {
                    self.log(Kind::Info, "Settings saved");
                }
                self.ui.set_form_error(SharedString::new());
            }
            Err(e) => self.ui.set_form_error(e.into()),
        }
        self.update_save_btn();
    }

    /// Returns whether anything changed.
    fn apply(&self, s: Settings) -> bool {
        let (changed, restart) = {
            let st = self.state.borrow();
            (st.settings != s, st.want && st.settings.listener_differs(&s))
        };
        if !changed {
            return false;
        }
        self.save(&s);
        self.state.borrow_mut().settings = s;
        if restart {
            self.stop_bridge();
            self.state.borrow_mut().retrying = false;
            self.try_start();
            self.refresh_status();
        }
        true
    }

    /// Enable Save only while the form differs from what's saved (or doesn't
    /// parse, so clicking it explains why). A fixed field clears the error.
    fn update_save_btn(&self) {
        let dirty = match self.collect_form() {
            Ok(s) => {
                self.ui.set_form_error(SharedString::new());
                s != self.state.borrow().settings
            }
            Err(_) => true,
        };
        self.ui.set_dirty(dirty);
    }

    fn refresh_devices(&self) {
        let names = match bridge::input_device_names() {
            Ok(n) => n,
            Err(e) => {
                self.log(Kind::Error, &format!("Couldn't list devices: {e}"));
                return;
            }
        };
        let current = usize::try_from(self.ui.get_device_index())
            .ok()
            .and_then(|i| self.devices.borrow().get(i).cloned());
        let want = current.unwrap_or_else(|| self.state.borrow().settings.device.clone());
        let lw = want.to_lowercase();
        let mut sel = names.iter().position(|n| *n == want)
            .or_else(|| names.iter().position(|n| n.to_lowercase().contains(&lw)));
        let mut names = names;
        if sel.is_none() && !want.is_empty() {
            names.push(want); // keep the saved choice even while it's unplugged
            sel = Some(names.len() - 1);
        }
        let model: Vec<SharedString> = names.iter().map(|n| n.as_str().into()).collect();
        self.ui.set_devices(ModelRc::new(VecModel::from(model)));
        self.ui.set_device_index(sel.map_or(-1, |i| i as i32));
        *self.devices.borrow_mut() = names;
    }

    fn record_clicked(&self) {
        if self.recording.get() {
            keys::cancel_record();
            return;
        }
        let done = |combo: Option<String>| {
            // Runs inside the keyboard hook; finish once it has returned.
            Timer::single_shot(Duration::ZERO, move || with_app(|a| a.end_record(combo)));
        };
        if let Err(e) = keys::record(done) {
            self.log(Kind::Error, &format!("Couldn't record a shortcut: {e}"));
            return;
        }
        *self.hotkey_before.borrow_mut() = self.ui.get_hotkey().to_string();
        self.recording.set(true);
        self.ui.set_recording(true);
        self.ui.set_hotkey("Press the shortcut... (Esc cancels)".into());
    }

    fn end_record(&self, combo: Option<String>) {
        self.recording.set(false);
        self.ui.set_recording(false);
        match combo {
            Some(c) => {
                self.ui.set_hotkey(c.as_str().into());
                self.log(Kind::Info, &format!("Shortcut set to {c}"));
                self.apply_form();
            }
            None => self.ui.set_hotkey(self.hotkey_before.borrow().as_str().into()),
        }
    }

    fn toggle_startup(&self, on: bool) {
        match util::set_start_with_windows(on) {
            Ok(()) if on => self.log(Kind::Info, "Will start with Windows, in the tray"),
            Ok(()) => self.log(Kind::Info, "Won't start with Windows"),
            Err(e) => {
                self.log(Kind::Error, &format!("Couldn't change start with Windows: {e}"));
                self.ui.set_startup(!on);
            }
        }
    }

    fn open_log(&self) {
        let path = data_dir().join("handy_shortcut.log");
        if let Err(e) = std::process::Command::new("explorer").arg(&path).spawn() {
            self.log(Kind::Error, &format!("Couldn't open {}: {e}", path.display()));
        }
    }

    // ---- window -------------------------------------------------------------

    fn show(&self) {
        if let Err(e) = self.ui.show() {
            self.log(Kind::Error, &format!("Couldn't show the window: {e}"));
            return;
        }
        finish_show(10);
        if !self.meter_timer.running() {
            self.meter_timer.start(TimerMode::Repeated, Duration::from_millis(50), || {
                with_app(|a| a.update_meter())
            });
        }
        self.scroll_log_soon();
    }

    /// The window is closing to the tray.
    fn hide(&self) {
        if self.recording.get() {
            keys::cancel_record();
        }
        self.apply_form();
        self.meter_timer.stop();
        self.ui.set_level(0.0);
        self.ui.set_level_text(SharedString::new());
    }

    fn quit(&self) {
        if self.recording.get() {
            keys::cancel_record();
        }
        self.apply_form();
        self.stop_bridge();
        keys::let_go();
        let _ = self.ui.hide();
        let _ = slint::quit_event_loop();
    }

    pub fn shutdown(&self) {
        self.stop_bridge();
        keys::let_go();
        let _ = self.tray.set_visible(false);
        APP.with(|a| a.borrow_mut().take());
    }

    fn update_meter(&self) {
        let peak = self.state.borrow().bridge.as_ref().map(|b| b.take_peak());
        match peak {
            Some(p) => {
                let db = 20.0 * p.max(1e-10).log10();
                self.ui.set_level(((db + 90.0) / 90.0).clamp(0.0, 1.0));
                self.ui.set_level_text(format!("{db:.1} dB").into());
            }
            None => {
                self.ui.set_level(0.0);
                self.ui.set_level_text(SharedString::new());
            }
        }
    }

    fn refresh_status(&self) {
        let (want, running, down, hotkey, mode) = {
            let st = self.state.borrow();
            let down = st.bridge.as_ref().is_some_and(|b| b.button_down());
            (st.want, st.bridge.is_some(), down, st.settings.hotkey.clone(), st.settings.mode)
        };
        let (text, kind, icon) = if running && down && mode == Mode::Hold {
            (format!("Holding {hotkey}"), 2, &self.tray_icons.holding)
        } else if running && down {
            ("Button down".to_string(), 2, &self.tray_icons.holding)
        } else if running {
            ("Listening".to_string(), 1, &self.tray_icons.listening)
        } else if want {
            ("Waiting for the microphone...".to_string(), 3, &self.tray_icons.error)
        } else {
            ("Stopped".to_string(), 0, &self.tray_icons.stopped)
        };
        self.ui.set_listening(want);
        self.mi_listen.set_checked(want);
        if self.state.borrow().shown == text {
            return;
        }
        self.ui.set_status_text(text.as_str().into());
        self.ui.set_status_kind(kind);
        let _ = self.tray.set_icon(Some(icon.clone()));
        let _ = self.tray.set_tooltip(Some(format!("{APP_NAME} - {text}")));
        self.state.borrow_mut().shown = text;
    }

    // ---- log ----------------------------------------------------------------

    fn log(&self, kind: Kind, text: &str) {
        let (date, time) = util::now();
        if let Some(f) = self.logf.borrow_mut().as_mut() {
            let _ = writeln!(f, "{date} [{time}] {text}");
        }
        if self.log.row_count() >= MAX_LOG_LINES {
            let keep: Vec<LogLine> = self.log.iter().skip(100).collect();
            self.log.set_vec(keep);
        }
        self.log.push(LogLine { time: time.into(), text: text.into(), kind: kind_code(kind) });
        self.scroll_log_soon();
    }

    /// Scroll the activity list to the newest line once it's been laid out.
    fn scroll_log_soon(&self) {
        Timer::single_shot(Duration::from_millis(10), || {
            with_app(|a| a.ui.invoke_scroll_log_to_end())
        });
    }
}

/// Set the window's icon and bring it to the front. The native window only
/// exists once the event loop has handled the show, so look for it from
/// there, a few times if need be.
fn finish_show(tries: u32) {
    Timer::single_shot(Duration::from_millis(50), move || {
        match util::own_window(APP_NAME) {
            Some(hwnd) => {
                util::set_window_icon(hwnd);
                util::bring_to_front(hwnd);
            }
            None if tries > 1 => finish_show(tries - 1),
            None => {}
        }
    });
}

fn open_log_file() -> Option<File> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("handy_shortcut.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_FILE) {
        let _ = std::fs::rename(&path, dir.join("handy_shortcut.old.log"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}
