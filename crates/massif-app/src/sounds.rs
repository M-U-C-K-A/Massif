//! Little sounds for what happens: a chime when a long task is done, two
//! notes when the map is saved, a low pair when something fails; and, when
//! asked for, soft ticks for the interface itself. Synthesised when Massif
//! starts — soft mallets, clicks and air, as Cuelume makes them — so there
//! is no sound file to ship, and played on a thread of their own.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

/// Which sounds play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundLevel {
    Off,
    /// What ends and what fails: tasks done, saving, errors.
    Essential,
    /// And the interface: choices, switches, windows, workspaces.
    All,
}

/// A sound, by what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cue {
    /// A long task is done: an export, an import, a conversion, a pass over
    /// the whole map.
    Ready,
    /// The map is saved.
    Saved,
    /// Something failed.
    Error,
    /// Something was refused or stopped.
    Warning,
    /// A choice among several: a tool, a segment.
    Select,
    /// A switch turned on or off.
    Toggle,
    /// A window opens.
    Open,
    /// A window closes.
    Close,
    /// Another workspace, or the other side of Massif.
    Navigate,
    /// A button.
    Tap,
}

impl Cue {
    pub const ALL: [Cue; 10] = [Cue::Ready, Cue::Saved, Cue::Error, Cue::Warning, Cue::Select, Cue::Toggle, Cue::Open, Cue::Close, Cue::Navigate, Cue::Tap];

    /// Whether it plays at the Essential level.
    pub const fn essential(self) -> bool {
        matches!(self, Cue::Ready | Cue::Saved | Cue::Error | Cue::Warning)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Cue::Ready => "Task done",
            Cue::Saved => "Saved",
            Cue::Error => "Error",
            Cue::Warning => "Refused or stopped",
            Cue::Select => "Choice",
            Cue::Toggle => "Switch",
            Cue::Open => "Window opens",
            Cue::Close => "Window closes",
            Cue::Navigate => "Workspace",
            Cue::Tap => "Button",
        }
    }
}

/// The level and volume (0–1) sounds play at, as the settings say.
static LEVEL: AtomicU8 = AtomicU8::new(1);
static VOLUME: AtomicU32 = AtomicU32::new(0x3f19_999a); // 0.6

/// Take the settings: the level and the volume.
pub fn configure(level: SoundLevel, volume: f32) {
    LEVEL.store(level as u8, Ordering::Relaxed);
    VOLUME.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// Play `cue`, if the level of the settings lets it.
pub fn play(cue: Cue) {
    let level = LEVEL.load(Ordering::Relaxed);
    let allowed = match level {
        0 => false,
        1 => cue.essential(),
        _ => true,
    };
    if allowed {
        preview(cue);
    }
}

/// Play `cue` whatever the level (to hear it from the settings).
pub fn preview(cue: Cue) {
    if cfg!(test) {
        return;
    }
    let volume = f32::from_bits(VOLUME.load(Ordering::Relaxed));
    if volume <= 0.0 {
        return;
    }
    static PLAYER: OnceLock<Sender<(Cue, f32)>> = OnceLock::new();
    let tx = PLAYER.get_or_init(|| {
        let (tx, rx) = channel::<(Cue, f32)>();
        let _ = std::thread::Builder::new().name("massif sounds".into()).spawn(move || {
            // The device is open only while sounds play, and a moment
            // after: an open output, even silent, holds the speakers — and
            // wireless headphones, which then stay with the Mac.
            let mut sink: Option<rodio::MixerDeviceSink> = None;
            // When what was played ends.
            let mut ends = Instant::now();
            let mut failed: Option<Instant> = None;
            let rate = 48_000u32;
            let samples: Vec<(Cue, Vec<f32>)> = Cue::ALL.iter().map(|c| (*c, synth(*c, rate as f32))).collect();
            loop {
                let next = if sink.is_some() {
                    match rx.recv_timeout(ends.saturating_duration_since(Instant::now()) + LINGER) {
                        Ok(next) => next,
                        Err(RecvTimeoutError::Timeout) => {
                            sink = None;
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(next) => next,
                        Err(_) => break,
                    }
                };
                let (cue, volume) = next;
                // A device that would not open is tried again later.
                if sink.is_none() && failed.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                    match open_output() {
                        Ok(mut s) => {
                            s.log_on_drop(false);
                            sink = Some(s);
                            failed = None;
                        }
                        Err(e) => {
                            log::warn!("no sound: {e}");
                            failed = Some(Instant::now());
                        }
                    }
                }
                let (Some(sink), Some((_, data))) = (&sink, samples.iter().find(|(c, _)| *c == cue)) else { continue };
                let data: Vec<f32> = data.iter().map(|s| s * volume).collect();
                let length = Duration::from_secs_f64(data.len() as f64 / rate as f64);
                let buffer = rodio::buffer::SamplesBuffer::new(std::num::NonZero::new(1).unwrap(), std::num::NonZero::new(rate).unwrap(), data);
                sink.mixer().add(buffer);
                ends = ends.max(Instant::now() + length);
            }
        });
        tx
    });
    let _ = tx.send((cue, volume));
}

/// How long the output stays open after the last sound ends, for the next
/// one to come without opening it again.
const LINGER: Duration = Duration::from_secs(2);

/// The default output, with a short buffer (some devices take no more
/// than 960 samples, and a cue should not wait), or any setting it takes.
fn open_output() -> Result<rodio::MixerDeviceSink, rodio::DeviceSinkError> {
    rodio::DeviceSinkBuilder::from_default_device()?
        .with_buffer_size(rodio::cpal::BufferSize::Fixed(512))
        .open_stream()
        .or_else(|_| rodio::DeviceSinkBuilder::from_default_device()?.open_sink_or_fallback())
}

// ------------------------------------------------------------- synthesis --

/// The samples of `cue` at `rate` per second, mono, within −1..1.
pub fn synth(cue: Cue, rate: f32) -> Vec<f32> {
    let mut out = Vec::new();
    // Notes: (when, frequency, length, loudness), soft mallets.
    let notes = |out: &mut Vec<f32>, notes: &[(f32, f32, f32, f32)], body: f32| {
        for &(at, f, len, gain) in notes {
            mallet(out, rate, at, f, len, gain, body);
        }
    };
    match cue {
        // Three notes rising: done.
        Cue::Ready => notes(&mut out, &[(0.0, 1046.5, 0.5, 0.22), (0.075, 1318.5, 0.5, 0.2), (0.15, 1568.0, 0.7, 0.22)], 0.12),
        // Two notes, a fifth up: kept.
        Cue::Saved => notes(&mut out, &[(0.0, 784.0, 0.35, 0.2), (0.07, 1174.7, 0.45, 0.2)], 0.1),
        // Two low notes falling, a little hollow: no.
        Cue::Error => notes(&mut out, &[(0.0, 329.6, 0.45, 0.32), (0.14, 261.6, 0.6, 0.32)], 0.4),
        // One note, held a moment, wavering.
        Cue::Warning => {
            notes(&mut out, &[(0.0, 440.0, 0.5, 0.26)], 0.3);
            tremolo(&mut out, rate, 9.0, 0.35);
        }
        Cue::Select => notes(&mut out, &[(0.0, 1318.5, 0.12, 0.14)], 0.05),
        Cue::Toggle => {
            click(&mut out, rate, 0.0, 0.006, 0.12);
            notes(&mut out, &[(0.0, 1046.5, 0.07, 0.1), (0.035, 1396.9, 0.08, 0.1)], 0.0);
        }
        Cue::Open => {
            air(&mut out, rate, 0.0, 0.12, true, 0.1);
            notes(&mut out, &[(0.05, 880.0, 0.18, 0.1)], 0.05);
        }
        Cue::Close => {
            air(&mut out, rate, 0.0, 0.1, false, 0.08);
            notes(&mut out, &[(0.02, 659.3, 0.14, 0.09)], 0.05);
        }
        Cue::Navigate => notes(&mut out, &[(0.0, 987.8, 0.1, 0.11), (0.04, 1318.5, 0.12, 0.1)], 0.03),
        Cue::Tap => click(&mut out, rate, 0.0, 0.012, 0.16),
    }
    // A breath of silence after, and nothing above full scale.
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.95 {
        for s in &mut out {
            *s *= 0.95 / peak;
        }
    }
    out
}

/// Add `len` samples at `at` seconds.
fn ensure(out: &mut Vec<f32>, n: usize) {
    if out.len() < n {
        out.resize(n, 0.0);
    }
}

/// A soft mallet: a sine and a few overtones, struck and dying away; `body`
/// adds the hollow odd overtones of a wooden bar.
fn mallet(out: &mut Vec<f32>, rate: f32, at: f32, f: f32, len: f32, gain: f32, body: f32) {
    let start = (at * rate) as usize;
    let n = (len * rate) as usize;
    ensure(out, start + n);
    let tau = std::f32::consts::TAU;
    for k in 0..n {
        let t = k as f32 / rate;
        let attack = (t / 0.004).min(1.0);
        let decay = (-t * 5.5 / len).exp();
        let fast = (-t * 18.0 / len).exp();
        let s = (tau * f * t).sin() + 0.18 * fast * (tau * 2.0 * f * t).sin() + body * 0.25 * fast * (tau * 3.0 * f * t).sin() + 0.05 * fast * (tau * 4.01 * f * t).sin();
        out[start + k] += gain * attack * decay * s;
    }
}

/// A click: a short burst of noise, bright, gone at once.
fn click(out: &mut Vec<f32>, rate: f32, at: f32, len: f32, gain: f32) {
    let start = (at * rate) as usize;
    let n = (len * rate) as usize;
    ensure(out, start + n);
    let mut seed = 0x9e37_79b9u32;
    let mut last = 0.0f32;
    for k in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let white = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
        // High-passed: a tick, not a thump.
        let high = white - last;
        last = white;
        let t = k as f32 / n as f32;
        out[start + k] += gain * high * (1.0 - t).powi(3);
    }
}

/// Air: noise swelling and passing, its colour rising (or falling).
fn air(out: &mut Vec<f32>, rate: f32, at: f32, len: f32, rising: bool, gain: f32) {
    let start = (at * rate) as usize;
    let n = (len * rate) as usize;
    ensure(out, start + n);
    let mut seed = 0x85eb_ca77u32;
    let mut low = 0.0f32;
    for k in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let white = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
        let t = k as f32 / n as f32;
        let colour = if rising { 0.04 + 0.3 * t } else { 0.34 - 0.3 * t };
        low += colour * (white - low);
        let swell = (std::f32::consts::PI * t).sin().powi(2);
        out[start + k] += gain * low * swell * 2.0;
    }
}

/// A wavering of loudness, `speed` times a second, `depth` deep.
fn tremolo(out: &mut [f32], rate: f32, speed: f32, depth: f32) {
    for (k, s) in out.iter_mut().enumerate() {
        let t = k as f32 / rate;
        *s *= 1.0 - depth * 0.5 * (1.0 + (std::f32::consts::TAU * speed * t).sin());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default output opens (nothing is played). Run by hand:
    /// `cargo test -p massif-app -- --ignored the_output_opens`.
    #[test]
    #[ignore]
    fn the_output_opens() {
        let mut sink = open_output().expect("an output device");
        sink.log_on_drop(false);
        eprintln!("{:?}", sink.config());
    }

    #[test]
    fn every_cue_sounds_short_and_clean() {
        for cue in Cue::ALL {
            let s = synth(cue, 48_000.0);
            let secs = s.len() as f32 / 48_000.0;
            assert!((0.005..1.2).contains(&secs), "{cue:?}: {secs} s");
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.02 && peak <= 0.95, "{cue:?}: peak {peak}");
            // It ends quietly, without a click.
            let tail = s[s.len() - s.len() / 20..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(tail < 0.12, "{cue:?}: tail {tail}");
        }
    }
}
