//! The settings window and tray icon.
//!
//! Closing the window hides it to the tray; Quit is in the tray menu. While
//! hidden nothing runs on the UI thread: the meter timer only ticks when the
//! window is visible, and status changes arrive through a Notice.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use native_windows_gui as nwg;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, SendMessageW, SetForegroundWindow, ShowWindow, ICON_SMALL, SW_RESTORE,
    WM_CHAR, WM_ENDSESSION, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_QUERYENDSESSION, WM_SETICON,
    WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::bridge::{self, Bridge, Kind, Sink};
use crate::keys::{self, Hotkey};
use crate::settings::{data_dir, Settings};
use crate::{icons, util, APP_NAME};

const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_GETLINECOUNT: u32 = 0x00BA;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SETLIMITTEXT: u32 = 0x00C5;

const MAX_LOG_LINES: isize = 500;
const MAX_LOG_FILE: u64 = 1 << 20;

#[derive(Default)]
struct State {
    settings: Settings,
    bridge: Option<Bridge>,
    /// The user wants it listening (it may be waiting for the mic).
    want: bool,
    /// Starting failed and the retry timer is running.
    retrying: bool,
    /// What the status label/tray currently show, to skip no-op updates.
    shown: String,
}

#[derive(Default)]
pub struct App {
    window: nwg::Window,
    title_font: nwg::Font,
    header_font: nwg::Font,
    mono_font: nwg::Font,
    labels: Vec<nwg::Label>,

    status: nwg::Label,
    level: nwg::ProgressBar,
    db: nwg::Label,
    run_btn: nwg::Button,

    hotkey: nwg::TextInput,
    rec_btn: nwg::Button,
    device: nwg::ComboBox<String>,
    refresh_btn: nwg::Button,
    channel: nwg::TextInput,
    invert: nwg::CheckBox,
    autostart: nwg::CheckBox,
    startup: nwg::CheckBox,
    peak: nwg::TextInput,
    width: nwg::TextInput,
    log_box: nwg::TextBox,

    tray: nwg::TrayNotification,
    menu: nwg::Menu,
    mi_open: nwg::MenuItem,
    mi_listen: nwg::MenuItem,
    mi_sep: nwg::MenuSeparator,
    mi_quit: nwg::MenuItem,

    notice: nwg::Notice,
    meter_timer: nwg::AnimationTimer,
    retry_timer: nwg::AnimationTimer,
    icons: icons::Icons,

    state: RefCell<State>,
    logf: RefCell<Option<File>>,
    events: Arc<Mutex<VecDeque<(Kind, String)>>>,
    recording: Cell<bool>,
    hotkey_before: RefCell<String>,
    handlers: RefCell<Vec<nwg::EventHandler>>,
    raw_handlers: RefCell<Vec<nwg::RawEventHandler>>,
}

fn checked(b: bool) -> nwg::CheckBoxState {
    if b { nwg::CheckBoxState::Checked } else { nwg::CheckBoxState::Unchecked }
}

fn is_checked(c: &nwg::CheckBox) -> bool {
    c.check_state() == nwg::CheckBoxState::Checked
}

fn label(
    parent: &nwg::Window,
    out: &mut Vec<nwg::Label>,
    text: &str,
    pos: (i32, i32),
    size: (i32, i32),
    font: Option<&nwg::Font>,
) -> Result<(), nwg::NwgError> {
    let mut l = nwg::Label::default();
    nwg::Label::builder().parent(parent).text(text).position(pos).size(size).font(font).build(&mut l)?;
    out.push(l);
    Ok(())
}

fn hwnd(handle: &nwg::ControlHandle) -> windows_sys::Win32::Foundation::HWND {
    handle.hwnd().map(|h| h as _).unwrap_or(std::ptr::null_mut())
}

impl App {
    pub fn build(tray_only: bool) -> Result<Rc<App>, nwg::NwgError> {
        let settings = Settings::load();
        let mut app = App::default();
        app.icons = icons::load();

        let mut font = nwg::Font::default();
        nwg::Font::builder().family("Segoe UI").size(16).build(&mut font)?;
        nwg::Font::set_global_default(Some(font));
        nwg::Font::builder().family("Segoe UI").size(24).weight(700).build(&mut app.title_font)?;
        nwg::Font::builder().family("Segoe UI").size(17).weight(700).build(&mut app.header_font)?;
        nwg::Font::builder().family("Consolas").size(14).build(&mut app.mono_font)?;

        nwg::Window::builder()
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::MINIMIZE_BOX)
            .size((560, 616))
            .center(true)
            .title(APP_NAME)
            .icon(Some(&app.icons.app))
            .build(&mut app.window)?;
        let w = &app.window;

        // status + level
        nwg::Label::builder().parent(w).text("Stopped").position((16, 12)).size((440, 30))
            .font(Some(&app.title_font)).build(&mut app.status)?;
        nwg::ProgressBar::builder().parent(w).position((16, 48)).size((360, 14)).range(0..90)
            .build(&mut app.level)?;
        nwg::Label::builder().parent(w).text("").position((384, 44)).size((76, 22))
            .h_align(nwg::HTextAlign::Right).build(&mut app.db)?;
        nwg::Button::builder().parent(w).text("Start").position((470, 16)).size((74, 44))
            .build(&mut app.run_btn)?;

        // settings
        let hf = Some(&app.header_font);
        label(w, &mut app.labels, "Settings", (16, 80), (300, 24), hf)?;
        label(w, &mut app.labels, "Shortcut to hold", (16, 113), (130, 22), None)?;
        nwg::TextInput::builder().parent(w).text(&settings.hotkey).position((150, 110))
            .size((300, 26)).build(&mut app.hotkey)?;
        nwg::Button::builder().parent(w).text("Record...").position((458, 109)).size((86, 28))
            .build(&mut app.rec_btn)?;

        label(w, &mut app.labels, "Microphone", (16, 149), (130, 22), None)?;
        nwg::ComboBox::builder().parent(w).position((150, 146)).size((300, 28))
            .build(&mut app.device)?;
        nwg::Button::builder().parent(w).text("Refresh").position((458, 145)).size((86, 28))
            .build(&mut app.refresh_btn)?;

        label(w, &mut app.labels, "Channel", (16, 185), (130, 22), None)?;
        nwg::TextInput::builder().parent(w).text(&settings.channel.to_string())
            .position((150, 182)).size((50, 26)).build(&mut app.channel)?;

        nwg::CheckBox::builder().parent(w).text("Swap press / release (if they come out backwards)")
            .position((16, 218)).size((528, 24)).check_state(checked(settings.invert))
            .build(&mut app.invert)?;
        nwg::CheckBox::builder().parent(w).text("Start listening when the app opens")
            .position((16, 244)).size((528, 24)).check_state(checked(settings.autostart))
            .build(&mut app.autostart)?;
        nwg::CheckBox::builder().parent(w).text("Start with Windows (in the tray)")
            .position((16, 270)).size((528, 24)).check_state(checked(util::starts_with_windows()))
            .build(&mut app.startup)?;

        // advanced detection tuning
        label(w, &mut app.labels, "Thump detection (advanced)", (16, 310), (400, 24), hf)?;
        label(w, &mut app.labels, "Min peak (0-1)", (16, 343), (130, 22), None)?;
        nwg::TextInput::builder().parent(w).text(&settings.peak_min.to_string())
            .position((150, 340)).size((60, 26)).build(&mut app.peak)?;
        label(w, &mut app.labels, "Min width (ms)", (240, 343), (110, 22), None)?;
        nwg::TextInput::builder().parent(w).text(&settings.width_ms.to_string())
            .position((356, 340)).size((60, 26)).build(&mut app.width)?;
        label(
            w,
            &mut app.labels,
            "Your button's thumps: peak 1.00, width ~2.7 ms. Ignored spikes in the log show \
             what speech looks like.",
            (16, 372),
            (528, 40),
            None,
        )?;

        // log
        label(w, &mut app.labels, "Activity", (16, 416), (300, 24), hf)?;
        nwg::TextBox::builder().parent(w).position((16, 442)).size((528, 160))
            .font(Some(&app.mono_font)).readonly(true)
            .flags(nwg::TextBoxFlags::VISIBLE | nwg::TextBoxFlags::VSCROLL | nwg::TextBoxFlags::AUTOVSCROLL)
            .build(&mut app.log_box)?;

        // tray
        nwg::TrayNotification::builder().parent(w).icon(Some(&app.icons.stopped))
            .tip(Some(APP_NAME)).build(&mut app.tray)?;
        nwg::Menu::builder().popup(true).parent(w).build(&mut app.menu)?;
        nwg::MenuItem::builder().text("Open Handy Shortcut").parent(&app.menu).build(&mut app.mi_open)?;
        nwg::MenuItem::builder().text("Listening").parent(&app.menu).build(&mut app.mi_listen)?;
        nwg::MenuSeparator::builder().parent(&app.menu).build(&mut app.mi_sep)?;
        nwg::MenuItem::builder().text("Quit").parent(&app.menu).build(&mut app.mi_quit)?;

        nwg::Notice::builder().parent(w).build(&mut app.notice)?;
        nwg::AnimationTimer::builder().parent(w).interval(Duration::from_millis(50))
            .build(&mut app.meter_timer)?;
        nwg::AnimationTimer::builder().parent(w).interval(Duration::from_secs(3))
            .build(&mut app.retry_timer)?;

        *app.logf.borrow_mut() = open_log_file();
        app.state.borrow_mut().settings = settings.clone();

        let app = Rc::new(app);
        app.bind_events();
        unsafe {
            SendMessageW(hwnd(&app.log_box.handle), EM_SETLIMITTEXT, 0, 0);
            SendMessageW(hwnd(&app.window.handle), WM_SETICON, ICON_SMALL as usize,
                         app.icons.app_small.handle as isize);
        }

        util::refresh_startup_path();
        app.refresh_devices();
        app.log(Kind::Info, &format!("{APP_NAME} {} started", env!("CARGO_PKG_VERSION")));
        if settings.autostart {
            app.state.borrow_mut().want = true;
            app.try_start();
        }
        app.refresh_status();
        if !tray_only {
            app.show();
        }
        Ok(app)
    }

    fn bind_events(self: &Rc<Self>) {
        use nwg::Event as E;

        let weak = Rc::downgrade(self);
        let handler = nwg::full_bind_event_handler(&self.window.handle, move |evt, data, handle| {
            let Some(app) = weak.upgrade() else { return };
            match evt {
                E::OnWindowClose if handle == app.window.handle => {
                    if let nwg::EventData::OnWindowClose(d) = &data {
                        d.close(false);
                    }
                    app.hide();
                }
                E::OnButtonClick => {
                    if handle == app.run_btn.handle {
                        app.toggle();
                    } else if handle == app.rec_btn.handle {
                        app.begin_record();
                    } else if handle == app.refresh_btn.handle {
                        app.refresh_devices();
                    } else if handle == app.invert.handle || handle == app.autostart.handle {
                        app.apply_form();
                    } else if handle == app.startup.handle {
                        app.toggle_startup();
                    }
                }
                E::OnComboxBoxSelection if handle == app.device.handle => app.apply_form(),
                E::OnContextMenu if handle == app.tray.handle => app.show_menu(),
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == app.tray.handle => {
                    app.show()
                }
                E::OnMenuItemSelected => {
                    if handle == app.mi_open.handle {
                        app.show();
                    } else if handle == app.mi_listen.handle {
                        app.toggle();
                    } else if handle == app.mi_quit.handle {
                        app.quit();
                    }
                }
                E::OnNotice if handle == app.notice.handle => app.drain_events(),
                E::OnTimerTick => {
                    if handle == app.meter_timer.handle {
                        app.update_meter();
                    } else if handle == app.retry_timer.handle {
                        app.retry();
                    }
                }
                _ => {}
            }
        });
        self.handlers.borrow_mut().push(handler);

        // Let go of the shortcut on log-off/shutdown, and show the window when
        // a second copy is started.
        let weak = Rc::downgrade(self);
        let show_msg = util::show_message();
        let raw = nwg::bind_raw_event_handler(&self.window.handle, 0x10001, move |_, msg, _, _| {
            if msg == WM_QUERYENDSESSION || msg == WM_ENDSESSION {
                keys::let_go();
            } else if msg == show_msg {
                if let Some(app) = weak.upgrade() {
                    app.show();
                }
                return Some(0);
            }
            None
        });
        // Record a shortcut by pressing it while the shortcut box has focus.
        let weak = Rc::downgrade(self);
        let rec = nwg::bind_raw_event_handler(&self.hotkey.handle, 0x10002, move |_, msg, wparam, _| {
            let app = weak.upgrade()?;
            if !app.recording.get() {
                return None;
            }
            match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let vk = wparam as u16;
                    if vk == 0x1B {
                        app.end_record(None);
                    } else if !keys::is_modifier(vk) {
                        app.end_record(Some(keys::combo_with(vk)));
                    }
                    Some(0)
                }
                WM_CHAR | WM_SYSCHAR | WM_KEYUP | WM_SYSKEYUP => Some(0),
                WM_KILLFOCUS => {
                    app.end_record(None);
                    None
                }
                _ => None,
            }
        });
        self.raw_handlers.borrow_mut().extend(raw.ok().into_iter().chain(rec.ok()));
    }

    // ---- listening ----------------------------------------------------------

    fn sink(&self) -> Sink {
        let queue = self.events.clone();
        let notice = self.notice.sender();
        Arc::new(move |kind, text| {
            queue.lock().unwrap_or_else(|e| e.into_inner()).push_back((kind, text));
            notice.notice();
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
        let Some(s) = self.collect_form(false) else { return };
        self.save(&s);
        {
            let mut st = self.state.borrow_mut();
            st.settings = s;
            st.want = true;
        }
        self.try_start();
        self.refresh_status();
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
                self.retry_timer.start();
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

    /// Settings from the form, or None (with an error box unless quiet).
    fn collect_form(&self, quiet: bool) -> Option<Settings> {
        let cur = self.state.borrow().settings.clone();
        let error = |msg: &str| {
            if !quiet {
                nwg::modal_error_message(&self.window, APP_NAME, msg);
            }
        };
        let channel = self.channel.text().trim().parse::<u32>();
        let peak = self.peak.text().trim().parse::<f32>();
        let width = self.width.text().trim().parse::<f32>();
        let (Ok(channel), Ok(peak_min), Ok(width_ms)) = (channel, peak, width) else {
            error("Channel, min peak and min width must be numbers.");
            return None;
        };
        let hotkey = if self.recording.get() {
            cur.hotkey.clone()
        } else {
            self.hotkey.text().trim().to_lowercase()
        };
        if let Err(e) = Hotkey::parse(&hotkey) {
            error(&format!(
                "'{hotkey}' isn't a shortcut I can press: {e}.\n\
                 Use something like ctrl+space or ctrl+shift+f9."
            ));
            return None;
        }
        Some(Settings {
            device: self.device.selection_string().unwrap_or_else(|| cur.device.clone()),
            channel: channel.max(1),
            hotkey,
            invert: is_checked(&self.invert),
            autostart: is_checked(&self.autostart),
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

    /// Save the form and, if listening, restart with it.
    fn apply_form(&self) {
        let Some(s) = self.collect_form(true) else { return };
        let (changed, restart) = {
            let st = self.state.borrow();
            (st.settings != s, st.want && st.settings.listener_differs(&s))
        };
        if !changed {
            return;
        }
        self.save(&s);
        self.state.borrow_mut().settings = s;
        if restart {
            self.stop_bridge();
            self.state.borrow_mut().retrying = false;
            self.try_start();
            self.refresh_status();
        }
    }

    fn refresh_devices(&self) {
        let names = match bridge::input_device_names() {
            Ok(n) => n,
            Err(e) => {
                self.log(Kind::Error, &format!("Couldn't list devices: {e}"));
                return;
            }
        };
        let want = self
            .device
            .selection_string()
            .unwrap_or_else(|| self.state.borrow().settings.device.clone());
        let lw = want.to_lowercase();
        let mut sel = names.iter().position(|n| *n == want)
            .or_else(|| names.iter().position(|n| n.to_lowercase().contains(&lw)));
        let mut names = names;
        if sel.is_none() && !want.is_empty() {
            names.push(want); // keep the saved choice even while it's unplugged
            sel = Some(names.len() - 1);
        }
        self.device.set_collection(names);
        self.device.set_selection(sel);
    }

    fn begin_record(&self) {
        *self.hotkey_before.borrow_mut() = self.hotkey.text();
        self.recording.set(true);
        self.hotkey.set_text("Press the shortcut... (Esc cancels)");
        self.hotkey.set_focus();
        self.rec_btn.set_enabled(false);
    }

    fn end_record(&self, combo: Option<String>) {
        self.recording.set(false);
        self.rec_btn.set_enabled(true);
        match combo {
            Some(c) => {
                self.hotkey.set_text(&c);
                self.log(Kind::Info, &format!("Shortcut set to {c}"));
                self.apply_form();
            }
            None => self.hotkey.set_text(&self.hotkey_before.borrow()),
        }
    }

    fn toggle_startup(&self) {
        let on = is_checked(&self.startup);
        match util::set_start_with_windows(on) {
            Ok(()) if on => self.log(Kind::Info, "Will start with Windows, in the tray"),
            Ok(()) => self.log(Kind::Info, "Won't start with Windows"),
            Err(e) => {
                self.log(Kind::Error, &format!("Couldn't change start with Windows: {e}"));
                self.startup.set_check_state(checked(!on));
            }
        }
    }

    // ---- window -------------------------------------------------------------

    fn show(&self) {
        self.window.set_visible(true);
        let h = hwnd(&self.window.handle);
        unsafe {
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
        self.meter_timer.start();
    }

    fn hide(&self) {
        if self.recording.get() {
            self.end_record(None);
        }
        self.apply_form();
        self.window.set_visible(false);
        self.meter_timer.stop();
        self.level.set_pos(0);
        self.db.set_text("");
    }

    fn show_menu(&self) {
        unsafe { SetForegroundWindow(hwnd(&self.window.handle)) };
        let (x, y) = nwg::GlobalCursor::position();
        self.menu.popup(x, y);
    }

    fn quit(&self) {
        self.apply_form();
        self.stop_bridge();
        keys::let_go();
        nwg::stop_thread_dispatch();
    }

    pub fn shutdown(&self) {
        self.stop_bridge();
        keys::let_go();
    }

    fn update_meter(&self) {
        let peak = self.state.borrow().bridge.as_ref().map(|b| b.take_peak());
        match peak {
            Some(p) => {
                let db = 20.0 * p.max(1e-10).log10();
                self.level.set_pos((db + 90.0).clamp(0.0, 90.0) as u32);
                self.db.set_text(&format!("{db:.1} dB"));
            }
            None => {
                self.level.set_pos(0);
                self.db.set_text("");
            }
        }
    }

    fn refresh_status(&self) {
        let (want, running, hotkey) = {
            let st = self.state.borrow();
            (st.want, st.bridge.is_some(), st.settings.hotkey.clone())
        };
        let (text, icon) = if running && keys::holding() {
            (format!("Holding {hotkey}"), &self.icons.holding)
        } else if running {
            ("Listening".to_string(), &self.icons.listening)
        } else if want {
            ("Waiting for the microphone...".to_string(), &self.icons.error)
        } else {
            ("Stopped".to_string(), &self.icons.stopped)
        };
        if self.state.borrow().shown == text {
            return;
        }
        self.status.set_text(&text);
        self.tray.set_icon(icon);
        self.tray.set_tip(&format!("{APP_NAME} - {text}"));
        self.run_btn.set_text(if want { "Stop" } else { "Start" });
        self.mi_listen.set_checked(want);
        self.state.borrow_mut().shown = text;
    }

    // ---- log ----------------------------------------------------------------

    fn log(&self, _kind: Kind, text: &str) {
        let (date, time) = util::now();
        let line = format!("[{time}] {text}");
        if let Some(f) = self.logf.borrow_mut().as_mut() {
            let _ = writeln!(f, "{date} {line}");
        }
        let h = hwnd(&self.log_box.handle);
        let replace = |s: &str| {
            let w = util::wide(s);
            unsafe { SendMessageW(h, EM_REPLACESEL, 0, w.as_ptr() as isize) };
        };
        unsafe {
            if SendMessageW(h, EM_GETLINECOUNT, 0, 0) > MAX_LOG_LINES {
                let cut = SendMessageW(h, EM_LINEINDEX, 100, 0);
                SendMessageW(h, EM_SETSEL, 0, cut);
                replace("");
            }
            let len = GetWindowTextLengthW(h);
            SendMessageW(h, EM_SETSEL, len as usize, len as isize);
            replace(&format!("{line}\r\n"));
            SendMessageW(h, EM_SCROLLCARET, 0, 0);
        }
    }
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
