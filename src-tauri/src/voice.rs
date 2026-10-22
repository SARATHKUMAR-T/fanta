//! Voice, fully local: the microphone via `arecord`, speech-to-text with
//! whisper.cpp, and text-to-speech with Piper. Models download on first use.

use futures_util::StreamExt;
use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{db, AppState};

const SAMPLE_RATE: usize = 16_000;
const FRAME: usize = 320; // 20 ms
/// Stop listening after this much quiet once speech has started.
const END_SILENCE: Duration = Duration::from_millis(1200);
/// Give up if nothing is said at all.
const NO_SPEECH_TIMEOUT: Duration = Duration::from_secs(7);
const MAX_UTTERANCE: Duration = Duration::from_secs(30);

const SPEAK_MODE: &str = "voice_speak"; // "voice" (only after voice input), "always", "never"

struct Asset {
    url: &'static str,
    file: &'static str,
}

const WHISPER_MODEL: Asset = Asset {
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
    file: "ggml-base.en.bin",
};
const PIPER: Asset = Asset {
    url: "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz",
    file: "piper_linux_x86_64.tar.gz",
};
const VOICE: Asset = Asset {
    url: "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/amy/medium/en_US-amy-medium.onnx",
    file: "en_US-amy-medium.onnx",
};
const VOICE_CONFIG: Asset = Asset {
    url: "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/amy/medium/en_US-amy-medium.onnx.json",
    file: "en_US-amy-medium.onnx.json",
};

fn dir() -> PathBuf {
    db::data_dir().join("voice")
}
fn whisper_path() -> PathBuf {
    dir().join(WHISPER_MODEL.file)
}
fn piper_bin() -> PathBuf {
    dir().join("piper").join("piper")
}
fn voice_path() -> PathBuf {
    dir().join(VOICE.file)
}

#[derive(Default)]
pub struct Voice {
    installing: AtomicBool,
    listening: AtomicBool,
    /// Set to end the current recording early (the user clicked stop).
    stop_listening: AtomicBool,
    speaker: Mutex<Option<(Child, Child)>>,
}

#[derive(Serialize)]
pub struct Status {
    pub installed: bool,
    pub installing: bool,
    pub listening: bool,
    pub speaking: bool,
    /// "voice", "always", or "never"
    pub speak_mode: String,
    pub download_mb: u32,
}

pub fn installed() -> bool {
    whisper_path().exists() && piper_bin().exists() && voice_path().exists() && dir().join(VOICE_CONFIG.file).exists()
}

pub fn status(app: &AppHandle) -> Status {
    let state = app.state::<AppState>();
    let v = &state.voice;
    let speak_mode = db::get_setting(&state.db.lock().unwrap(), SPEAK_MODE).ok().flatten().unwrap_or_else(|| "voice".into());
    let speaking = v.speaker.lock().unwrap().is_some();
    Status {
        installed: installed(),
        installing: v.installing.load(Ordering::SeqCst),
        listening: v.listening.load(Ordering::SeqCst),
        speaking,
        speak_mode,
        download_mb: 230,
    }
}

pub fn set_speak_mode(app: &AppHandle, mode: &str) -> anyhow::Result<()> {
    if !["voice", "always", "never"].contains(&mode) {
        anyhow::bail!("unknown mode {mode}");
    }
    db::set_setting(&app.state::<AppState>().db.lock().unwrap(), SPEAK_MODE, mode)
}

// ---------------- install ----------------

async fn download(app: &AppHandle, asset: &Asset, label: &str) -> anyhow::Result<()> {
    let target = dir().join(asset.file);
    if target.exists() {
        return Ok(());
    }
    let resp = app.state::<AppState>().http.get(asset.url).send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let partial = target.with_extension("part");
    let mut file = std::fs::File::create(&partial)?;
    let mut done: u64 = 0;
    let mut last = Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(250) {
            last = Instant::now();
            let _ = app.emit("voice://progress", serde_json::json!({ "label": label, "done": done, "total": total }));
        }
    }
    file.flush()?;
    std::fs::rename(partial, target)?;
    Ok(())
}

/// Downloads the speech models (~230 MB) once.
pub async fn install(app: &AppHandle) -> anyhow::Result<()> {
    let voice = &app.state::<AppState>().voice;
    if voice.installing.swap(true, Ordering::SeqCst) {
        anyhow::bail!("already downloading");
    }
    let result = async {
        std::fs::create_dir_all(dir())?;
        download(app, &PIPER, "speech engine").await?;
        if !piper_bin().exists() {
            let ok = Command::new("tar").arg("xzf").arg(dir().join(PIPER.file)).arg("-C").arg(dir()).status()?.success();
            if !ok {
                anyhow::bail!("couldn't unpack the speech engine");
            }
        }
        download(app, &VOICE_CONFIG, "Fanta's voice").await?;
        download(app, &VOICE, "Fanta's voice").await?;
        download(app, &WHISPER_MODEL, "speech recognition").await?;
        anyhow::Ok(())
    }
    .await;
    voice.installing.store(false, Ordering::SeqCst);
    let _ = app.emit("voice://changed", ());
    result
}

// ---------------- listen ----------------

fn whisper() -> anyhow::Result<&'static Mutex<WhisperContext>> {
    static CTX: OnceLock<Mutex<WhisperContext>> = OnceLock::new();
    if let Some(c) = CTX.get() {
        return Ok(c);
    }
    let ctx = WhisperContext::new_with_params(whisper_path(), WhisperContextParameters::default())
        .map_err(|e| anyhow::anyhow!("couldn't load speech recognition: {e}"))?;
    Ok(CTX.get_or_init(|| Mutex::new(ctx)))
}

fn rms(frame: &[i16]) -> f32 {
    let sum: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / frame.len().max(1) as f64).sqrt() as f32
}

/// Reads 16 kHz mono s16 audio until the speaker pauses. Timing is counted in
/// samples, so it behaves the same on live audio and in tests.
fn capture(mut input: impl Read, stop: &AtomicBool, mut on_level: impl FnMut(f32)) -> anyhow::Result<Vec<i16>> {
    let ms = |n: usize| n * 1000 / SAMPLE_RATE; // samples -> milliseconds
    let mut samples: Vec<i16> = Vec::with_capacity(SAMPLE_RATE * 10);
    let mut buf = [0u8; FRAME * 2];
    let mut noise_floor = 0f32;
    let mut speech_start: Option<usize> = None;
    let mut last_voice = 0usize;
    let mut frames = 0usize;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(samples);
        }
        if input.read_exact(&mut buf).is_err() {
            if speech_start.is_some() {
                return Ok(samples);
            }
            anyhow::bail!("the microphone stopped");
        }
        let frame: Vec<i16> = buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let level = rms(&frame);
        samples.extend_from_slice(&frame);
        frames += 1;
        if frames % 4 == 0 {
            on_level((level / 4000.0).min(1.0));
        }
        let now = samples.len();
        // Learn the room's background level from the first 300 ms.
        if ms(now) < 300 {
            noise_floor = noise_floor.max(level);
            continue;
        }
        if level > (noise_floor * 2.5).max(400.0) {
            last_voice = now;
            speech_start.get_or_insert(now);
        }
        match speech_start {
            None if ms(now) as u128 > NO_SPEECH_TIMEOUT.as_millis() => anyhow::bail!("I didn't hear anything"),
            Some(_) if ms(now - last_voice) as u128 > END_SILENCE.as_millis() => return Ok(samples),
            Some(s) if ms(now - s) as u128 > MAX_UTTERANCE.as_millis() => return Ok(samples),
            _ => {}
        }
    }
}

/// Records from the default microphone until the speaker pauses. Returns 16 kHz mono samples.
fn record(app: &AppHandle, voice: &Voice) -> anyhow::Result<Vec<f32>> {
    let mut child = Command::new("arecord")
        .args(["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("couldn't open the microphone: {e}"))?;
    let out = child.stdout.take().expect("piped stdout");
    let result = capture(out, &voice.stop_listening, |level| {
        let _ = app.emit("voice://level", level);
    });
    let _ = child.kill();
    let _ = child.wait();
    Ok(result?.iter().map(|&s| s as f32 / 32768.0).collect())
}

fn transcribe(samples: &[f32]) -> anyhow::Result<String> {
    let ctx = whisper()?.lock().unwrap();
    let mut state = ctx.create_state().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_n_threads(std::thread::available_parallelism().map(|n| n.get().min(8) as i32).unwrap_or(4));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    state.full(params, samples).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut text = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(seg) = state.get_segment(i) {
            text.push_str(&seg.to_str_lossy().map_err(|e| anyhow::anyhow!("{e}"))?);
        }
    }
    // Whisper marks non-speech like "[BLANK_AUDIO]" or "(wind blowing)".
    let cleaned: String = text
        .split(|c| c == '[' || c == '(')
        .enumerate()
        .map(|(i, part)| if i == 0 { part } else { part.split_once([']', ')']).map(|(_, rest)| rest).unwrap_or("") })
        .collect();
    Ok(cleaned.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Listens for one utterance and emits `voice://heard` with the text.
pub fn start_listening(app: &AppHandle) -> anyhow::Result<()> {
    if !installed() {
        anyhow::bail!("voice isn't set up yet");
    }
    let state = app.state::<AppState>();
    if state.voice.listening.swap(true, Ordering::SeqCst) {
        // Already listening: a second press means "I'm done talking".
        state.voice.stop_listening.store(true, Ordering::SeqCst);
        return Ok(());
    }
    stop_speaking(app);
    state.voice.stop_listening.store(false, Ordering::SeqCst);
    let _ = app.emit("voice://state", "listening");
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let voice = &app.state::<AppState>().voice;
        let result = record(&app, voice).and_then(|samples| {
            voice.listening.store(false, Ordering::SeqCst);
            let _ = app.emit("voice://state", "transcribing");
            transcribe(&samples)
        });
        voice.listening.store(false, Ordering::SeqCst);
        match result {
            Ok(text) if !text.is_empty() => {
                let _ = app.emit("voice://heard", text);
            }
            Ok(_) => {
                let _ = app.emit("voice://error", "I didn't catch that");
            }
            Err(e) => {
                let _ = app.emit("voice://error", e.to_string());
            }
        }
        let _ = app.emit("voice://state", "idle");
    });
    Ok(())
}

// ---------------- speak ----------------

/// Makes chat text pleasant to hear: no links, emoji, or markdown symbols.
pub fn speakable(text: &str) -> String {
    let no_links: String = text.split_whitespace().filter(|w| !w.starts_with("http")).collect::<Vec<_>>().join(" ");
    no_links
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || ".,!?;:'\"-()%&/".contains(*c))
        .collect::<String>()
        .replace(" - ", ", ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn sample_rate() -> u32 {
    std::fs::read_to_string(dir().join(VOICE_CONFIG.file))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["audio"]["sample_rate"].as_u64())
        .unwrap_or(22_050) as u32
}

pub fn speak(app: &AppHandle, text: &str) -> anyhow::Result<()> {
    if !installed() {
        anyhow::bail!("voice isn't set up yet");
    }
    let text = speakable(text);
    if text.is_empty() {
        return Ok(());
    }
    stop_speaking(app);
    // piper writes raw audio to stdout; aplay plays it through PipeWire's ALSA layer.
    let mut piper = Command::new(piper_bin())
        .arg("--model")
        .arg(voice_path())
        .arg("--output-raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("couldn't start the voice: {e}"))?;
    let audio = piper.stdout.take().expect("piped stdout");
    let player = Command::new("aplay")
        .args(["-q", "-r", &sample_rate().to_string(), "-f", "S16_LE", "-t", "raw", "-c", "1", "-"])
        .stdin(audio)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("couldn't play audio: {e}"))?;
    if let Some(mut stdin) = piper.stdin.take() {
        stdin.write_all(text.as_bytes())?;
        stdin.write_all(b"\n")?;
    } // dropping stdin closes it, so piper finishes after this text

    let state = app.state::<AppState>();
    *state.voice.speaker.lock().unwrap() = Some((piper, player));
    let _ = app.emit("voice://speaking", true);

    // Watch for playback to finish.
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(150));
        let voice = &app.state::<AppState>().voice;
        let mut guard = voice.speaker.lock().unwrap();
        let done = match guard.as_mut() {
            Some((_, player)) => !matches!(player.try_wait(), Ok(None)),
            None => true,
        };
        if done {
            if let Some((mut p, _)) = guard.take() {
                let _ = p.kill();
                let _ = p.wait();
            }
            drop(guard);
            let _ = app.emit("voice://speaking", false);
            break;
        }
    });
    Ok(())
}

pub fn stop_speaking(app: &AppHandle) {
    if let Some((mut piper, mut player)) = app.state::<AppState>().voice.speaker.lock().unwrap().take() {
        let _ = player.kill();
        let _ = piper.kill();
        let _ = player.wait();
        let _ = piper.wait();
    }
}

/// Whether a reply should be read aloud, given how the user asked.
pub fn should_speak(app: &AppHandle, by_voice: bool) -> bool {
    installed()
        && match status(app).speak_mode.as_str() {
            "always" => true,
            "never" => false,
            _ => by_voice,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(parts: &[(u32, i16)]) -> Vec<u8> {
        // (milliseconds, amplitude) pieces of a square-ish wave
        let mut out = vec![];
        for &(ms, amp) in parts {
            for i in 0..(ms as usize * SAMPLE_RATE / 1000) {
                let v = if (i / 20) % 2 == 0 { amp } else { -amp };
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn stops_after_a_pause() {
        // quiet room, 1.5 s of speech, then silence that goes on for 5 s
        let input = audio(&[(500, 30), (1500, 3000), (5000, 30)]);
        let samples = capture(&input[..], &AtomicBool::new(false), |_| {}).unwrap();
        let secs = samples.len() as f32 / SAMPLE_RATE as f32;
        assert!((3.1..3.4).contains(&secs), "stopped at {secs}s"); // 0.5 + 1.5 + ~1.2 pause
    }

    #[test]
    fn gives_up_when_nobody_talks() {
        let input = audio(&[(9000, 30)]);
        let err = capture(&input[..], &AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(err.to_string().contains("didn't hear"));
    }

    #[test]
    fn learns_a_noisy_room() {
        // a loud fan (amplitude 900) shouldn't count as speech; talking over it should
        let input = audio(&[(500, 900), (1000, 900), (1500, 5000), (3000, 900)]);
        let samples = capture(&input[..], &AtomicBool::new(false), |_| {}).unwrap();
        let secs = samples.len() as f32 / SAMPLE_RATE as f32;
        assert!((4.1..4.4).contains(&secs), "stopped at {secs}s");
    }

    #[test]
    fn speakable_text() {
        assert_eq!(
            speakable("📅 Quick sync in 5 min. Join: https://teams.microsoft.com/l/x **now** 😊"),
            "Quick sync in 5 min. Join: now"
        );
    }
}

#[cfg(test)]
mod real_tests {
    use super::*;

    /// Needs the downloaded models: `cargo test --release -- --ignored voice_round_trip --nocapture`
    #[test]
    #[ignore]
    fn voice_round_trip() {
        let said = "Hey Fanta, remind me at four to send the budget report.";
        let t = Instant::now();
        let mut piper = Command::new(piper_bin())
            .arg("--model").arg(voice_path()).arg("--output-raw")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().unwrap();
        piper.stdin.take().unwrap().write_all(said.as_bytes()).unwrap();
        let mut raw = vec![];
        piper.stdout.take().unwrap().read_to_end(&mut raw).unwrap();
        println!("tts: {} bytes in {:?}", raw.len(), t.elapsed());

        // 22.05 kHz -> 16 kHz for whisper
        let mut ff = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-f", "s16le", "-ar", &sample_rate().to_string(), "-ac", "1", "-i", "-", "-f", "s16le", "-ar", "16000", "-"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut stdin = ff.stdin.take().unwrap();
        std::thread::spawn(move || stdin.write_all(&raw).unwrap());
        let mut pcm = vec![];
        ff.stdout.take().unwrap().read_to_end(&mut pcm).unwrap();
        let samples: Vec<f32> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();

        let t = Instant::now();
        let heard = transcribe(&samples).unwrap();
        println!("stt: {heard:?} in {:?}", t.elapsed());
        assert!(heard.to_lowercase().contains("budget report"));
    }
}
