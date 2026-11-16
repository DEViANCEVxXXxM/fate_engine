//! Procedural one-shot audio output.
//!
//! The engine exposes a small, concrete capability: play a short synthesized
//! tone. Consumers decide which tones map to which events, so reusable crates
//! stay free of game-specific sound design. Tones are mixed on a bounded voice
//! pool with a short attack/release envelope to avoid clicks.
//!
//! A device is optional. [`AudioEngine::new`] reports an [`AudioError`] when no
//! output device or supported format exists, and callers can keep running
//! without audio. The mixer is covered by headless tests; opening a device is
//! never required for them.

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Maximum number of tones mixed at once.
///
/// When the pool is full, new tones are dropped rather than replacing a sound
/// that is already playing. Feedback sounds stay short, so this is only reached
/// under a burst of events.
pub const MAX_VOICES: usize = 16;

/// Longest accepted tone duration.
pub const MAX_TONE_SECONDS: f32 = 5.0;

/// Envelope attack/release used for every tone, in seconds.
const ENVELOPE_SECONDS: f32 = 0.004;

/// Oscillator shape for a [`Tone`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Square,
    Triangle,
    Noise,
}

/// One synthesized one-shot sound.
///
/// Values are sanitized when they enter the mixer: non-finite inputs become
/// silent, duration is capped at [`MAX_TONE_SECONDS`], amplitude is capped at
/// `1.0`, and playback frequency is limited below the device sample rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub frequency_hz: f32,
    pub duration_secs: f32,
    pub amplitude: f32,
    pub waveform: Waveform,
}

impl Tone {
    pub fn sine(frequency_hz: f32, duration_secs: f32, amplitude: f32) -> Self {
        Self {
            frequency_hz,
            duration_secs,
            amplitude,
            waveform: Waveform::Sine,
        }
    }

    pub fn square(frequency_hz: f32, duration_secs: f32, amplitude: f32) -> Self {
        Self {
            frequency_hz,
            duration_secs,
            amplitude,
            waveform: Waveform::Square,
        }
    }

    pub fn triangle(frequency_hz: f32, duration_secs: f32, amplitude: f32) -> Self {
        Self {
            frequency_hz,
            duration_secs,
            amplitude,
            waveform: Waveform::Triangle,
        }
    }

    /// A short noise burst; `frequency_hz` is ignored.
    pub fn noise(duration_secs: f32, amplitude: f32) -> Self {
        Self {
            frequency_hz: 0.0,
            duration_secs,
            amplitude,
            waveform: Waveform::Noise,
        }
    }

    /// Clamp tone parameters into the ranges the mixer accepts.
    pub fn sanitized(self) -> Self {
        Self {
            frequency_hz: finite_or(self.frequency_hz, 0.0).clamp(0.0, 20_000.0),
            duration_secs: finite_or(self.duration_secs, 0.0).clamp(0.0, MAX_TONE_SECONDS),
            amplitude: finite_or(self.amplitude, 0.0).clamp(0.0, 1.0),
            waveform: self.waveform,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    waveform: Waveform,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    elapsed: u32,
    duration: u32,
    attack: u32,
    release: u32,
    noise: u32,
}

impl Voice {
    fn from_tone(tone: Tone, sample_rate: u32, noise_seed: u32) -> Self {
        let duration = ((tone.duration_secs * sample_rate as f32).round() as u32).max(1);
        let envelope = ((ENVELOPE_SECONDS * sample_rate as f32).round() as u32)
            .max(1)
            .min(duration / 2);
        // Keep the limit at or above the lower bound so a pathological device
        // sample rate cannot make `clamp` panic.
        let max_frequency = (sample_rate as f32 * 0.45).max(20.0);
        let frequency = tone.frequency_hz.clamp(20.0, max_frequency);

        Self {
            waveform: tone.waveform,
            amplitude: tone.amplitude,
            phase: 0.0,
            phase_step: frequency / sample_rate as f32,
            elapsed: 0,
            duration,
            attack: envelope,
            release: envelope,
            noise: noise_seed | 1,
        }
    }

    fn envelope(&self) -> f32 {
        let attack = if self.attack == 0 {
            1.0
        } else {
            self.elapsed as f32 / self.attack as f32
        };
        let remaining = self.duration.saturating_sub(self.elapsed);
        let release = if self.release == 0 {
            1.0
        } else {
            remaining as f32 / self.release as f32
        };
        attack.min(release).clamp(0.0, 1.0)
    }

    fn next_sample(&mut self) -> f32 {
        let wave = match self.waveform {
            Waveform::Sine => (self.phase * std::f32::consts::TAU).sin(),
            Waveform::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            Waveform::Noise => self.next_noise(),
        };

        self.phase += self.phase_step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        wave * self.envelope() * self.amplitude
    }

    fn next_noise(&mut self) -> f32 {
        let mut state = self.noise;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        self.noise = state;
        (state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Advance the voice clock and report whether it is still audible.
    fn advance(&mut self) -> bool {
        self.elapsed += 1;
        self.elapsed < self.duration
    }
}

/// Bounded, allocation-free-while-rendering voice mixer.
struct Mixer {
    sample_rate: u32,
    voices: Vec<Voice>,
    noise_seed: u32,
    volume: f32,
}

impl Mixer {
    fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            voices: Vec::with_capacity(MAX_VOICES),
            noise_seed: 0x9E37_79B9,
            volume: 1.0,
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = finite_or(volume, 0.0).clamp(0.0, 1.0);
    }

    fn push(&mut self, tone: Tone) -> bool {
        let tone = tone.sanitized();
        if tone.duration_secs <= 0.0 || tone.amplitude <= 0.0 {
            return false;
        }
        if self.voices.len() >= MAX_VOICES {
            return false;
        }

        self.noise_seed = self
            .noise_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.voices
            .push(Voice::from_tone(tone, self.sample_rate, self.noise_seed));
        true
    }

    fn next_sample(&mut self) -> f32 {
        let mut output = 0.0;
        for voice in &mut self.voices {
            output += voice.next_sample();
        }
        self.voices.retain_mut(Voice::advance);
        (output * self.volume).clamp(-1.0, 1.0)
    }
}

/// An open output stream plus the mixer it renders from.
///
/// Dropping the engine stops playback and closes the device stream.
pub struct AudioEngine {
    _stream: cpal::Stream,
    mixer: Arc<Mutex<Mixer>>,
}

impl AudioEngine {
    /// Open the default output device and start rendering.
    ///
    /// This is the only operation that touches an audio device. Call it from a
    /// live application path, not from a headless test.
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoOutputDevice)?;
        let supported = device
            .default_output_config()
            .map_err(AudioError::DefaultConfig)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let mixer = Arc::new(Mutex::new(Mixer::new(config.sample_rate)));

        let stream = match format {
            SampleFormat::I8 => build_stream::<i8>(&device, &config, mixer.clone()),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, mixer.clone()),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, &config, mixer.clone()),
            SampleFormat::I32 => build_stream::<i32>(&device, &config, mixer.clone()),
            SampleFormat::I64 => build_stream::<i64>(&device, &config, mixer.clone()),
            SampleFormat::U8 => build_stream::<u8>(&device, &config, mixer.clone()),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, mixer.clone()),
            SampleFormat::U32 => build_stream::<u32>(&device, &config, mixer.clone()),
            SampleFormat::U64 => build_stream::<u64>(&device, &config, mixer.clone()),
            SampleFormat::F32 => build_stream::<f32>(&device, &config, mixer.clone()),
            SampleFormat::F64 => build_stream::<f64>(&device, &config, mixer.clone()),
            other => return Err(AudioError::UnsupportedSampleFormat(other)),
        }
        .map_err(AudioError::BuildStream)?;

        stream.play().map_err(AudioError::Play)?;

        Ok(Self {
            _stream: stream,
            mixer,
        })
    }

    /// Queue one shot. Tones either play completely or are dropped whole.
    ///
    /// Returns `false` when the tone is silent or the voice pool is full.
    pub fn play(&self, tone: Tone) -> bool {
        let mut mixer = self
            .mixer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        mixer.push(tone)
    }

    /// Set the master volume in `0..=1`.
    ///
    /// Non-finite values mute the output, and values outside the range are
    /// clamped. The change applies to the next rendered buffer, including tones
    /// that are already playing.
    pub fn set_volume(&self, volume: f32) {
        let mut mixer = self
            .mixer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        mixer.set_volume(volume);
    }
}

/// Failure to open or start audio output.
#[derive(Debug)]
pub enum AudioError {
    NoOutputDevice,
    DefaultConfig(cpal::Error),
    BuildStream(cpal::Error),
    Play(cpal::Error),
    UnsupportedSampleFormat(SampleFormat),
}

impl fmt::Display for AudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOutputDevice => formatter.write_str("no default audio output device"),
            Self::DefaultConfig(error) => write!(formatter, "cannot read output config: {error}"),
            Self::BuildStream(error) => write!(formatter, "cannot build output stream: {error}"),
            Self::Play(error) => write!(formatter, "cannot start output stream: {error}"),
            Self::UnsupportedSampleFormat(format) => {
                write!(formatter, "unsupported output sample format: {format}")
            }
        }
    }
}

impl Error for AudioError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DefaultConfig(error) | Self::BuildStream(error) | Self::Play(error) => {
                Some(error)
            }
            Self::NoOutputDevice | Self::UnsupportedSampleFormat(_) => None,
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_output(data, channels, &mixer);
        },
        |error| log::warn!(target: "fate.audio", "output stream error: {error}"),
        None,
    )
}

fn write_output<T>(data: &mut [T], channels: usize, mixer: &Mutex<Mixer>)
where
    T: Sample + FromSample<f32>,
{
    // Never block the audio thread on the producer. A missed update is a single
    // silent buffer, which is preferable to a stalled device.
    let Ok(mut mixer) = mixer.try_lock() else {
        data.fill(T::from_sample(0.0));
        return;
    };
    for frame in data.chunks_mut(channels) {
        let value = T::from_sample(mixer.next_sample());
        for sample in frame {
            *sample = value;
        }
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(mixer: &mut Mixer, samples: usize) -> Vec<f32> {
        (0..samples).map(|_| mixer.next_sample()).collect()
    }

    #[test]
    fn silence_is_produced_without_voices() {
        let mut mixer = Mixer::new(48_000);
        assert!(render(&mut mixer, 256).iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn tone_stops_after_its_duration() {
        let mut mixer = Mixer::new(1_000);
        assert!(mixer.push(Tone::sine(100.0, 0.1, 1.0)));

        let audible = render(&mut mixer, 100);
        let after = render(&mut mixer, 20);

        assert!(audible.iter().any(|sample| sample.abs() > 0.1));
        assert!(after.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn envelope_avoids_a_hard_start() {
        let mut mixer = Mixer::new(48_000);
        assert!(mixer.push(Tone::sine(440.0, 0.05, 1.0)));

        let first = mixer.next_sample();

        assert!(first.abs() < 0.01, "first sample {first} should fade in");
    }

    #[test]
    fn voice_pool_is_bounded() {
        let mut mixer = Mixer::new(48_000);
        for _ in 0..MAX_VOICES {
            assert!(mixer.push(Tone::sine(440.0, 0.1, 0.1)));
        }
        assert_eq!(mixer.voices.len(), MAX_VOICES);

        assert!(!mixer.push(Tone::sine(440.0, 0.1, 0.1)));
        assert_eq!(mixer.voices.len(), MAX_VOICES);
    }

    #[test]
    fn silent_tones_are_rejected() {
        let mut mixer = Mixer::new(48_000);
        assert!(!mixer.push(Tone::sine(440.0, 0.0, 1.0)));
        assert!(!mixer.push(Tone::sine(440.0, 0.1, 0.0)));
        assert!(mixer.voices.is_empty());
    }

    #[test]
    fn mixing_saturates_instead_of_clipping_past_full_scale() {
        let mut mixer = Mixer::new(48_000);
        for _ in 0..MAX_VOICES {
            mixer.push(Tone::square(220.0, 0.1, 1.0));
        }

        let peak = render(&mut mixer, 512)
            .into_iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));

        assert!(peak <= 1.0, "peak {peak} exceeded full scale");
    }

    #[test]
    fn noise_is_bounded_and_not_constant() {
        let mut mixer = Mixer::new(48_000);
        mixer.push(Tone::noise(0.05, 1.0));

        let samples = render(&mut mixer, 512);
        assert!(samples.iter().all(|sample| sample.abs() <= 1.0));
        assert!(samples.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn master_volume_scales_and_mutes_output() {
        let render_peak = |volume: f32| {
            let mut mixer = Mixer::new(48_000);
            mixer.set_volume(volume);
            mixer.push(Tone::square(220.0, 0.1, 1.0));
            render(&mut mixer, 512)
                .into_iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
        };

        let full = render_peak(1.0);
        let half = render_peak(0.5);
        let muted = render_peak(0.0);

        assert!(full > 0.0);
        assert!((half - full * 0.5).abs() < 1e-4);
        assert_eq!(muted, 0.0);
    }

    #[test]
    fn master_volume_clamps_and_mutes_non_finite_values() {
        let mut mixer = Mixer::new(48_000);

        mixer.set_volume(4.0);
        assert_eq!(mixer.volume, 1.0);

        mixer.set_volume(f32::NAN);
        assert_eq!(mixer.volume, 0.0);

        mixer.set_volume(-1.0);
        assert_eq!(mixer.volume, 0.0);
    }

    #[test]
    fn low_sample_rates_do_not_break_the_frequency_limit() {
        let mut mixer = Mixer::new(8);
        assert!(mixer.push(Tone::sine(440.0, 0.1, 1.0)));
        assert!(
            render(&mut mixer, 4)
                .iter()
                .all(|sample| sample.is_finite())
        );
    }

    #[test]
    fn tone_sanitization_clamps_and_rejects_non_finite_values() {
        let tone = Tone {
            frequency_hz: f32::NAN,
            duration_secs: 10.0,
            amplitude: 2.0,
            waveform: Waveform::Sine,
        }
        .sanitized();

        assert_eq!(tone.frequency_hz, 0.0);
        assert_eq!(tone.duration_secs, MAX_TONE_SECONDS);
        assert_eq!(tone.amplitude, 1.0);

        let silent = Tone::sine(440.0, f32::INFINITY, f32::NAN).sanitized();
        assert_eq!(silent.duration_secs, 0.0);
        assert_eq!(silent.amplitude, 0.0);
    }
}
