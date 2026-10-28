//! Listens to the mic and holds/releases the shortcut on button thumps.
//!
//! The audio callback finds spikes; a worker thread turns them into key
//! presses (so key timing doesn't depend on the UI being responsive) and
//! reports what happened through the event sink.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::detector::{Detector, Params, Spike};
use crate::keys::{self, Hotkey};
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Info,
    Down,
    Up,
    Ignored,
    Extra,
    Error,
    /// The audio stream broke (e.g. the device was unplugged).
    StreamError,
}

pub type Sink = Arc<dyn Fn(Kind, String) + Send + Sync>;

/// Input device names (WASAPI, shared mode, so Handy can use the same mic).
pub fn input_device_names() -> Result<Vec<String>, String> {
    let devices = cpal::default_host().input_devices().map_err(|e| e.to_string())?;
    let mut names: Vec<String> = Vec::new();
    for name in devices.filter_map(|d| d.name().ok()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

fn find_device(name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if name.trim().is_empty() {
        return host.default_input_device().ok_or_else(|| "no default input device".into());
    }
    let want = name.to_lowercase();
    host.input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().is_ok_and(|n| n.to_lowercase().contains(&want)))
        .ok_or_else(|| format!("no input device matching '{name}'"))
}

pub struct Bridge {
    stream: Option<cpal::Stream>,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Loudest sample since the meter last looked, as f32 bits.
    peak: Arc<AtomicU32>,
}

impl Bridge {
    pub fn start(s: &Settings, sink: Sink) -> Result<Bridge, String> {
        let hotkey = Hotkey::parse(&s.hotkey)?;
        let device = find_device(&s.device)?;
        let name = device.name().unwrap_or_else(|_| s.device.clone());
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let nch = (supported.channels() as usize).max(1);
        let ch = if nch == 1 { 0 } else { (s.channel.max(1) as usize).min(nch) - 1 };
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let params = Params {
            click_level: s.click_level,
            peak_min: s.peak_min,
            width_ms: s.width_ms,
            quiet_ms: s.quiet_ms,
            invert: s.invert,
        };
        let detector = Detector::new(params, config.sample_rate.0);
        let (tx, rx) = mpsc::channel::<Spike>();
        let peak = Arc::new(AtomicU32::new(0));
        let input = Input { ch, nch, detector, tx, peak: peak.clone() };

        let err_sink = sink.clone();
        let on_error = move |e: cpal::StreamError| err_sink(Kind::StreamError, e.to_string());
        let stream = match format {
            SampleFormat::F32 => input.build::<f32>(&device, &config, on_error),
            SampleFormat::F64 => input.build::<f64>(&device, &config, on_error),
            SampleFormat::I16 => input.build::<i16>(&device, &config, on_error),
            SampleFormat::I32 => input.build::<i32>(&device, &config, on_error),
            SampleFormat::U16 => input.build::<u16>(&device, &config, on_error),
            SampleFormat::I8 => input.build::<i8>(&device, &config, on_error),
            SampleFormat::U8 => input.build::<u8>(&device, &config, on_error),
            other => return Err(format!("unsupported sample format {other}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;

        let running = Arc::new(AtomicBool::new(true));
        let worker = {
            let running = running.clone();
            let sink = sink.clone();
            let label = s.hotkey.clone();
            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(spike) => handle(spike, &hotkey, &label, &sink),
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
        };

        sink(Kind::Info, format!("Listening on {name} (channel {}/{nch})", ch + 1));
        Ok(Bridge { stream: Some(stream), running, worker: Some(worker), peak })
    }

    /// Loudest sample since the last call.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        drop(self.stream.take());
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        keys::let_go();
    }
}

fn handle(spike: Spike, hotkey: &Hotkey, label: &str, sink: &Sink) {
    let info = format!(
        "{} spike, peak {:.2}, width {:.1} ms",
        if spike.up { '+' } else { '-' },
        spike.peak,
        spike.width_ms
    );
    if !spike.ok {
        sink(Kind::Ignored, format!("ignored {info}"));
    } else if spike.press && !keys::holding() {
        keys::hold(hotkey);
        sink(Kind::Down, format!("button DOWN -> holding {label}  ({info})"));
    } else if !spike.press && keys::holding() {
        keys::let_go();
        sink(Kind::Up, format!("button UP -> released  ({info})"));
    } else {
        let what = if spike.press { "press" } else { "release" };
        sink(Kind::Extra, format!("extra {what} thump, no change  ({info})"));
    }
}

/// Everything the audio callback owns.
struct Input {
    ch: usize,
    nch: usize,
    detector: Detector,
    tx: Sender<Spike>,
    peak: Arc<AtomicU32>,
}

impl Input {
    fn build<T>(
        mut self,
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        on_error: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let mut mono: Vec<f32> = Vec::with_capacity(4096);
        device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                mono.clear();
                mono.extend(data.iter().skip(self.ch).step_by(self.nch).map(|&s| f32::from_sample(s)));
                let loudest = mono.iter().fold(0f32, |m, &v| m.max(v.abs()));
                // Non-negative floats order the same as their bit patterns.
                self.peak.fetch_max(loudest.to_bits(), Ordering::Relaxed);
                let tx = &self.tx;
                self.detector.process(&mono, |spike| {
                    let _ = tx.send(spike);
                });
            },
            on_error,
            None,
        )
    }
}
