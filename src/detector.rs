//! Finds the "thump" a mic's push-to-talk button puts on the signal.
//!
//! Press and release thumps point in opposite directions (press jumps up
//! first, release jumps down first). Only spikes shaped like a button thump
//! count: near full scale and a couple of milliseconds wide, which loud
//! speech and plosives aren't.

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Sample level (0..1) where a spike starts.
    pub click_level: f32,
    /// A thump must reach this; real thumps clip at 1.0.
    pub peak_min: f32,
    /// ...and stay past `click_level` this long, in one direction.
    pub width_ms: f32,
    /// A new thump needs this much calm before it.
    pub quiet_ms: u32,
    /// Treat downward thumps as presses.
    pub invert: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Spike {
    /// Press (after applying `invert`), otherwise release.
    pub press: bool,
    /// The waveform jumped up first.
    pub up: bool,
    pub peak: f32,
    pub width_ms: f32,
    /// Shaped like a button thump.
    pub ok: bool,
}

pub struct Detector {
    p: Params,
    sample_rate: f32,
    quiet: i64,
    lookahead: usize,
    /// Samples not judged yet. A sample is judged once `lookahead` samples
    /// follow it, so a spike near the end of a block can still be measured.
    buf: Vec<f32>,
    /// Absolute index of `buf[0]`.
    start: i64,
    last_hit: i64,
}

impl Detector {
    pub fn new(p: Params, sample_rate: u32) -> Self {
        let quiet = (sample_rate as i64 * p.quiet_ms as i64) / 1000;
        Detector {
            p,
            sample_rate: sample_rate as f32,
            quiet,
            lookahead: (sample_rate as usize / 100).max(1), // 10 ms
            buf: Vec::with_capacity(sample_rate as usize / 10),
            start: 0,
            last_hit: -quiet,
        }
    }

    pub fn process(&mut self, samples: &[f32], mut emit: impl FnMut(Spike)) {
        self.buf.extend_from_slice(samples);
        let ready = self.buf.len().saturating_sub(self.lookahead);
        let level = self.p.click_level;
        for i in 0..ready {
            let v = self.buf[i];
            if v.abs() <= level {
                continue;
            }
            let pos = self.start + i as i64;
            if pos - self.last_hit >= self.quiet {
                let sign = if v > 0.0 { 1.0 } else { -1.0 };
                let seg = &self.buf[i..i + self.lookahead];
                let width = seg.iter().position(|&s| s * sign <= level).unwrap_or(seg.len());
                let width_ms = width as f32 / self.sample_rate * 1000.0;
                let peak = seg.iter().fold(f32::MIN, |m, &s| m.max(s * sign));
                let up = sign > 0.0;
                emit(Spike {
                    press: up != self.p.invert,
                    up,
                    peak,
                    width_ms,
                    ok: peak >= self.p.peak_min && width_ms >= self.p.width_ms,
                });
            }
            self.last_hit = pos;
        }
        self.buf.drain(..ready);
        self.start += ready as i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn params() -> Params {
        Params { click_level: 0.3, peak_min: 0.9, width_ms: 2.3, quiet_ms: 30, invert: false }
    }

    /// Silence with a pulse of `amp` lasting `ms` starting at 100 ms.
    fn signal(amp: f32, ms: f32) -> Vec<f32> {
        let mut x = vec![0.001f32; SR as usize / 2];
        let start = SR as usize / 10;
        let n = (SR as f32 * ms / 1000.0) as usize;
        for s in &mut x[start..start + n] {
            *s = amp;
        }
        x
    }

    fn run(x: &[f32], block: usize, p: Params) -> Vec<Spike> {
        let mut d = Detector::new(p, SR);
        let mut out = vec![];
        for chunk in x.chunks(block) {
            d.process(chunk, |s| out.push(s));
        }
        out
    }

    #[test]
    fn press_and_release_thumps() {
        let up = run(&signal(1.0, 2.7), 480, params());
        assert_eq!(up.len(), 1);
        assert!(up[0].ok && up[0].press && up[0].up);
        assert!((up[0].width_ms - 2.7).abs() < 0.1);

        let down = run(&signal(-1.0, 2.8), 480, params());
        assert_eq!(down.len(), 1);
        assert!(down[0].ok && !down[0].press && !down[0].up);
    }

    #[test]
    fn invert_swaps_press_and_release() {
        let p = Params { invert: true, ..params() };
        let s = run(&signal(1.0, 2.7), 480, p);
        assert!(s[0].ok && !s[0].press);
    }

    #[test]
    fn speech_like_spikes_are_ignored() {
        let short = run(&signal(1.0, 0.5), 480, params());
        assert_eq!(short.len(), 1);
        assert!(!short[0].ok);
        // A full-scale 2.0 ms spike seen during speech in a real log.
        let borderline = run(&signal(-1.0, 2.0), 480, params());
        assert!(!borderline[0].ok);
        let quiet = run(&signal(0.6, 3.0), 480, params());
        assert!(!quiet[0].ok);
    }

    #[test]
    fn block_size_does_not_matter() {
        let x = signal(1.0, 2.7);
        for block in [37, 128, 441, 480, 1024, 4800] {
            let s = run(&x, block, params());
            assert_eq!(s.len(), 1, "block {block}");
            assert!(s[0].ok, "block {block}");
        }
    }
}
