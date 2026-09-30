//! Microphone capture (P5 dev spec §4). The mic is open only between `start` and `stop`; the
//! audio stays in memory and is resampled to 16 kHz mono for whisper.

use std::io::Cursor;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rubato::Resampler;

use crate::error::{AppError, AppResult};

pub const TARGET_RATE: u32 = 16_000;
pub const MAX_LENGTH: Duration = Duration::from_secs(60);
/// Shorter clips are treated as "didn't hear anything".
pub const MIN_LENGTH: Duration = Duration::from_millis(300);
/// Quieter clips are treated as silence (≈ −50 dBFS).
pub const SILENCE_RMS: f32 = 0.003_16;
/// VAD auto-stop: speech is above −45 dBFS; stop after this much silence.
const VAD_SPEECH_RMS: f32 = 0.005_62;
const VAD_SILENCE: Duration = Duration::from_millis(1200);
const LEVEL_EVERY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct Clip {
    pub samples_16k_mono: Vec<f32>,
    pub duration: Duration,
    pub rms: f32,
    pub peak: f32,
}

impl Clip {
    /// Too short or too quiet to be speech.
    pub fn is_silence(&self) -> bool {
        self.duration < MIN_LENGTH || self.rms < SILENCE_RMS
    }
}

/// Why recording stopped on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStop {
    MaxLength,
    Silence,
}

impl AutoStop {
    pub fn as_str(self) -> &'static str {
        match self {
            AutoStop::MaxLength => "max_length",
            AutoStop::Silence => "silence",
        }
    }
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Average the channels of interleaved frames.
pub fn mix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resample mono audio to 16 kHz.
pub fn to_16k(mono: &[f32], rate: u32) -> AppResult<Vec<f32>> {
    if rate == TARGET_RATE || mono.is_empty() {
        return Ok(mono.to_vec());
    }
    let err = |e: &dyn std::fmt::Display| AppError::Internal(format!("resample: {e}"));
    let mut r = rubato::Fft::<f32>::new(rate as usize, TARGET_RATE as usize, 1024, 1, rubato::FixedSync::Both)
        .map_err(|e| err(&e))?;
    let data = vec![mono.to_vec()];
    let input =
        rubato::audioadapter_buffers::direct::SequentialSliceOfVecs::new(&data, 1, mono.len()).map_err(|e| err(&e))?;
    let out = r.process_all(&input, mono.len(), None).map_err(|e| err(&e))?;
    Ok(out.take_data())
}

/// 16-bit PCM mono WAV in memory.
pub fn encode_wav(samples_16k: &[f32]) -> AppResult<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cur = Cursor::new(Vec::with_capacity(samples_16k.len() * 2 + 44));
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).map_err(|e| AppError::Internal(format!("wav: {e}")))?;
        for s in samples_16k {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| AppError::Internal(format!("wav: {e}")))?;
        }
        w.finalize().map_err(|e| AppError::Internal(format!("wav: {e}")))?;
    }
    Ok(cur.into_inner())
}

// ---------------------------------------------------------------- permission (macOS)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPermission {
    NotDetermined,
    Denied,
    Granted,
}

#[cfg(target_os = "macos")]
pub fn mic_permission() -> MicPermission {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return MicPermission::Granted;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => MicPermission::Granted,
        AVAuthorizationStatus::NotDetermined => MicPermission::NotDetermined,
        _ => MicPermission::Denied,
    }
}

/// Show the macOS prompt (first use) and wait for the answer.
#[cfg(target_os = "macos")]
pub fn request_mic_permission() -> MicPermission {
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return MicPermission::Granted;
    };
    let (tx, rx) = mpsc::channel::<bool>();
    let tx = Mutex::new(Some(tx));
    let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(granted.as_bool());
        }
    });
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(true) => MicPermission::Granted,
        Ok(false) => MicPermission::Denied,
        Err(_) => MicPermission::NotDetermined,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn mic_permission() -> MicPermission {
    MicPermission::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn request_mic_permission() -> MicPermission {
    MicPermission::Granted
}

pub fn mic_denied() -> AppError {
    AppError::ai(
        "mic_denied",
        "Tech English can't use the microphone. Allow it in System Settings › Privacy & Security › Microphone, then try again.",
    )
}

/// Blocking: ask for permission if macOS hasn't asked yet.
pub fn ensure_mic_permission() -> AppResult<()> {
    let p = match mic_permission() {
        MicPermission::NotDetermined => request_mic_permission(),
        p => p,
    };
    match p {
        MicPermission::Granted => Ok(()),
        _ => Err(mic_denied()),
    }
}

// ---------------------------------------------------------------- recording

struct Shared {
    /// Mono samples at the device rate.
    samples: Vec<f32>,
    error: Option<String>,
}

/// An open microphone. Dropping it closes the mic too.
pub struct Recording {
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Mutex<Shared>>,
    rate: u32,
    started: Instant,
}

pub type LevelFn = Box<dyn Fn(f32) + Send>;
pub type AutoStopFn = Box<dyn FnOnce(AutoStop) + Send>;

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let err_shared = shared.clone();
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            let f: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
            let mono = mix_to_mono(&f, channels);
            if let Ok(mut g) = shared.lock() {
                g.samples.extend_from_slice(&mono);
            }
        },
        move |e: cpal::Error| {
            tracing::warn!(error = %e, "microphone stream error");
            if let Ok(mut g) = err_shared.lock() {
                g.error.get_or_insert_with(|| e.to_string());
            }
        },
        None,
    )
}

impl Recording {
    /// Open the default input device. `on_level` gets the RMS every 50 ms; `on_auto_stop` fires
    /// once when the clip hits 60 s or (with `vad`) after a pause. The caller still calls `stop`.
    pub fn start(vad: bool, on_level: LevelFn, on_auto_stop: AutoStopFn) -> AppResult<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            samples: Vec::new(),
            error: None,
        }));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<AppResult<u32>>();
        let sh = shared.clone();
        // cpal streams are not Send on every platform: the stream lives on this thread only.
        let thread = std::thread::Builder::new()
            .name("mic".into())
            .spawn(move || {
                let host = cpal::default_host();
                let Some(device) = host.default_input_device() else {
                    let _ = ready_tx.send(Err(AppError::ai("no_device", "No microphone found.")));
                    return;
                };
                let supported = match device.default_input_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(map_cpal(&e)));
                        return;
                    }
                };
                let rate = supported.sample_rate();
                let config: cpal::StreamConfig = supported.config();
                let stream = match supported.sample_format() {
                    SampleFormat::F32 => build::<f32>(&device, config, sh.clone()),
                    SampleFormat::I16 => build::<i16>(&device, config, sh.clone()),
                    SampleFormat::I32 => build::<i32>(&device, config, sh.clone()),
                    SampleFormat::U16 => build::<u16>(&device, config, sh.clone()),
                    other => {
                        let _ = ready_tx.send(Err(AppError::Invalid(format!(
                            "The microphone's sample format ({other}) is not supported."
                        ))));
                        return;
                    }
                };
                let stream = match stream.and_then(|s| s.play().map(|_| s)) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(map_cpal(&e)));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(rate));
                monitor(&sh, rate, vad, &stop_rx, on_level, on_auto_stop);
                drop(stream); // closes the mic (the menu-bar indicator goes off)
            })
            .map_err(|e| AppError::Internal(format!("mic thread: {e}")))?;
        let rate = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| AppError::Internal("the microphone did not start".into()))??;
        Ok(Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
            shared,
            rate,
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Close the mic and return the clip (16 kHz mono).
    pub fn stop(mut self) -> AppResult<Clip> {
        self.close();
        let mono = std::mem::take(&mut self.shared.lock().unwrap().samples);
        let max = (MAX_LENGTH.as_secs_f64() * self.rate as f64) as usize;
        let mono = &mono[..mono.len().min(max)];
        let duration = Duration::from_secs_f64(mono.len() as f64 / self.rate as f64);
        let samples = to_16k(mono, self.rate)?;
        Ok(Clip {
            rms: rms(&samples),
            peak: samples.iter().fold(0.0f32, |m, s| m.max(s.abs())),
            samples_16k_mono: samples,
            duration,
        })
    }

    fn close(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.close();
    }
}

fn map_cpal(e: &cpal::Error) -> AppError {
    match e.kind() {
        cpal::ErrorKind::PermissionDenied => mic_denied(),
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable => {
            AppError::ai("no_device", "No microphone found.")
        }
        _ => AppError::Internal(format!("microphone: {e}")),
    }
}

/// Runs on the mic thread until `stop_rx` fires: level meter, 60 s cap, optional VAD stop.
fn monitor(
    shared: &Mutex<Shared>,
    rate: u32,
    vad: bool,
    stop_rx: &mpsc::Receiver<()>,
    on_level: LevelFn,
    on_auto_stop: AutoStopFn,
) {
    let mut seen = 0usize;
    let mut auto = Some(on_auto_stop);
    let mut heard_speech = false;
    let mut quiet_since: Option<Instant> = None;
    loop {
        match stop_rx.recv_timeout(LEVEL_EVERY) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let (level, total) = {
            let g = shared.lock().unwrap();
            let level = rms(&g.samples[seen.min(g.samples.len())..]);
            (level, g.samples.len())
        };
        seen = total;
        on_level(level);
        let fire = if total as f64 / rate as f64 >= MAX_LENGTH.as_secs_f64() {
            Some(AutoStop::MaxLength)
        } else if vad {
            if level >= VAD_SPEECH_RMS {
                heard_speech = true;
                quiet_since = None;
                None
            } else if heard_speech {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                (since.elapsed() >= VAD_SILENCE).then_some(AutoStop::Silence)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(reason) = fire
            && let Some(f) = auto.take()
        {
            f(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32, amp: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn resampled_length_matches_the_duration() {
        for rate in [48_000, 44_100, 16_000] {
            let out = to_16k(&sine(rate, 2.5, 0.5), rate).unwrap();
            let want = 2.5 * 16_000.0;
            assert!(
                (out.len() as f32 - want).abs() <= 16.0,
                "{rate}: {} vs {want}",
                out.len()
            );
            // The tone keeps its level (RMS of a sine = amp / √2).
            assert!((rms(&out) - 0.5 / 2f32.sqrt()).abs() < 0.02, "{rate}");
        }
    }

    #[test]
    fn stereo_is_averaged() {
        assert_eq!(mix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(mix_to_mono(&[0.25], 1), vec![0.25]);
    }

    #[test]
    fn wav_header_and_length() {
        let wav = encode_wav(&[0.0, 0.5, -0.5, 1.0]).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 4 * 2);
        let r = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!((r.spec().sample_rate, r.spec().channels), (16_000, 1));
    }

    #[test]
    fn silence_rules() {
        let clip = |secs: f32, amp: f32| {
            let s = sine(16_000, secs, amp);
            Clip {
                rms: rms(&s),
                peak: amp,
                duration: Duration::from_secs_f32(secs),
                samples_16k_mono: s,
            }
        };
        assert!(clip(0.2, 0.5).is_silence(), "too short");
        assert!(clip(2.0, 0.002).is_silence(), "too quiet (≈ −57 dBFS)");
        assert!(!clip(2.0, 0.2).is_silence());
    }
}
