//! Sonor listener — hears the machine room.
//!
//! Captures the audio the PC is playing, runs it through the Sonor analyser and
//! streams one JSON frame per analysis window on stdout. The Node server pipes
//! those frames to every browser connected to /listen.
//!
//! Backends:
//!   wasapi  (windows, default) — loopback capture of the default output device
//!   ffmpeg  — spawns ffmpeg with a dshow device (needs a capture device)
//!   file    — decodes a wav file (offline analysis / testing)

use sonor_core::analyze::Analyzer;
use std::io::{Read, Write};

const FRAME: usize = 2048;
const HOP: usize = 1024;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut backend = String::from("wasapi");
    let mut device = String::new();
    let mut file = String::new();
    let mut list_devices = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--list-devices" => list_devices = true,
            "--device" => {
                i += 1;
                device = args.get(i).cloned().unwrap_or_default();
                if !device.is_empty() {
                    backend = "ffmpeg".into();
                }
            }
            "--file" => {
                i += 1;
                file = args.get(i).cloned().unwrap_or_default();
                backend = "file".into();
            }
            "--backend" => {
                i += 1;
                backend = args.get(i).cloned().unwrap_or_else(|| "wasapi".into());
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => eprintln!("sonor-listener: ignoring unknown argument {:?}", other),
        }
        i += 1;
    }

    if list_devices {
        list_capture_devices();
        return;
    }

    let mut analyzer = Analyzer::new(FRAME);
    let mut analysis = sonor_core::params::SonorAnalysis::default();

    match backend.as_str() {
        "file" => {
            let path = if file.is_empty() { "input.wav".into() } else { file };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let (samples, sr) = decode_wav(&bytes);
                    eprintln!(
                        "sonor-listener: {} — {} samples @ {} Hz",
                        path,
                        samples.len(),
                        sr
                    );
                    let mut pos = 0usize;
                    while pos + FRAME <= samples.len() {
                        analyzer.analyze(&samples[pos..pos + FRAME], sr, HOP as f32, &mut analysis);
                        emit(&analysis);
                        pos += HOP;
                    }
                }
                Err(e) => eprintln!("sonor-listener: cannot read {}: {}", path, e),
            }
        }
        "ffmpeg" => {
            run_ffmpeg(&device, &mut analyzer, &mut analysis);
        }
        _ => {
            run_wasapi(&mut analyzer, &mut analysis);
        }
    }
}

fn print_usage() {
    eprintln!(
        "sonor-listener [--backend wasapi|ffmpeg|file] [--device <dshow name>] [--file <wav>] [--list-devices]"
    );
}

fn emit(a: &sonor_core::params::SonorAnalysis) {
    let mut out = String::with_capacity(512);
    out.push_str("{\"rms\":");
    out.push_str(&format!("{:.5}", a.rms));
    out.push_str(",\"peak\":");
    out.push_str(&format!("{:.5}", a.peak));
    out.push_str(",\"centroid\":");
    out.push_str(&format!("{:.2}", a.centroid));
    out.push_str(",\"flatness\":");
    out.push_str(&format!("{:.4}", a.flatness));
    out.push_str(",\"flux\":");
    out.push_str(&format!("{:.5}", a.flux));
    out.push_str(",\"bpm\":");
    out.push_str(&format!("{:.2}", a.bpm));
    out.push_str(",\"confidence\":");
    out.push_str(&format!("{:.4}", a.confidence));
    out.push_str(",\"clarity\":");
    out.push_str(&format!("{:.4}", a.clarity));
    out.push_str(",\"root\":");
    out.push_str(&a.root.to_string());
    out.push_str(",\"scale\":");
    out.push_str(&a.scale.to_string());
    out.push_str(",\"chroma\":[");
    for (k, c) in a.chroma.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        out.push_str(&format!("{:.4}", c));
    }
    out.push_str("],\"spectrum\":[");
    for (k, c) in a.spectrum.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        out.push_str(&format!("{:.4}", c));
    }
    out.push_str("]}\n");
    let mut so = std::io::stdout();
    let _ = so.write_all(out.as_bytes());
    let _ = so.flush();
}

/* ------------------------------------------------------------------ */
/* WAV decoding                                                        */
/* ------------------------------------------------------------------ */
fn decode_wav(bytes: &[u8]) -> (Vec<f32>, f32) {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        eprintln!("sonor-listener: not a RIFF/WAVE file");
        return (Vec::new(), 48_000.0);
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, f32)> = None; // (format, channels, rate, bits)
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + size).min(bytes.len());
        if id == b"fmt " && body_end - body_start >= 16 {
            let f = u16::from_le_bytes(bytes[body_start..body_start + 2].try_into().unwrap());
            let ch = u16::from_le_bytes(bytes[body_start + 2..body_start + 4].try_into().unwrap());
            let rate = u32::from_le_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap());
            let bits = u16::from_le_bytes(bytes[body_start + 14..body_start + 16].try_into().unwrap());
            fmt = Some((f, ch, rate, bits as f32));
        } else if id == b"data" {
            data = Some(&bytes[body_start..body_end]);
        }
        pos = body_start + size + (size & 1);
    }
    let (format, channels, rate, bits) = fmt.unwrap_or((1, 1, 48_000, 16.0));
    let data = data.unwrap_or(&[]);
    let mut samples = Vec::with_capacity(data.len() / 2);
    match (format, bits as u32) {
        (1, 16) => {
            let mut i = 0usize;
            while i + 2 <= data.len() {
                let v = i16::from_le_bytes(data[i..i + 2].try_into().unwrap()) as f32 / 32768.0;
                samples.push(v);
                i += 2;
            }
        }
        (1, 32) => {
            let mut i = 0usize;
            while i + 4 <= data.len() {
                let v = f32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                samples.push(v);
                i += 4;
            }
        }
        (3, 32) => {
            let mut i = 0usize;
            while i + 4 <= data.len() {
                let v = f32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                samples.push(v);
                i += 4;
            }
        }
        (1, 24) => {
            let mut i = 0usize;
            while i + 3 <= data.len() {
                let b = &data[i..i + 3];
                let v = ((b[0] as i32) | ((b[1] as i32) << 8) | ((b[2] as i32) << 16)) as i32;
                let v = (v << 8) >> 8;
                samples.push(v as f32 / 8_388_608.0);
                i += 3;
            }
        }
        _ => eprintln!(
            "sonor-listener: unsupported wav format {} / {} bits",
            format, bits
        ),
    }
    if channels > 1 {
        let mut mono = Vec::with_capacity(samples.len() / channels as usize);
        for chunk in samples.chunks(channels as usize) {
            mono.push(chunk.iter().sum::<f32>() / chunk.len() as f32);
        }
        samples = mono;
    }
    (samples, rate as f32)
}

/* ------------------------------------------------------------------ */
/* ffmpeg backend                                                      */
/* ------------------------------------------------------------------ */
fn run_ffmpeg(
    device: &str,
    analyzer: &mut Analyzer,
    analysis: &mut sonor_core::params::SonorAnalysis,
) {
    use std::process::{Command, Stdio};
    let ffmpeg = std::env::var("SONOR_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    if device.is_empty() {
        cmd.args(["-f", "dshow", "-i", "audio=virtual-audio-capturer"]);
    } else {
        cmd.args(["-f", "dshow", "-i", &format!("audio={}", device)]);
    }
    cmd.args(["-ac", "1", "-ar", "48000", "-f", "f32le", "-"]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sonor-listener: cannot spawn {}: {}", ffmpeg, e);
            return;
        }
    };
    let mut stdout = child.stdout.take().unwrap();
    let mut buf = vec![0u8; FRAME * 4];
    let mut carry: Vec<f32> = Vec::with_capacity(FRAME * 2);
    loop {
        let mut got = 0usize;
        while got < buf.len() {
            match stdout.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(_) => break,
            }
        }
        if got == 0 {
            break;
        }
        for c in buf[..got].chunks_exact(4) {
            carry.push(f32::from_le_bytes(c.try_into().unwrap()));
        }
        while carry.len() >= FRAME {
            analyzer.analyze(&carry[..FRAME], 48_000.0, HOP as f32, analysis);
            emit(analysis);
            carry.drain(0..HOP);
        }
    }
}

/* ------------------------------------------------------------------ */
/* wasapi backend (windows)                                            */
/* ------------------------------------------------------------------ */
fn list_capture_devices() {
    #[cfg(windows)]
    wasapi::list_devices();
    #[cfg(not(windows))]
    eprintln!("sonor-listener: device listing is only available on windows");
}

fn run_wasapi(analyzer: &mut Analyzer, analysis: &mut sonor_core::params::SonorAnalysis) {
    #[cfg(windows)]
    wasapi::capture_loop(analyzer, analysis);
    #[cfg(not(windows))]
    eprintln!("sonor-listener: wasapi capture is only available on windows");
}

#[cfg(windows)]
mod wasapi {
    use super::*;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// Loopback capture of the default render endpoint.
    ///
    /// WASAPI loopback gives us exactly what the speakers are playing, with no
    /// extra driver: the render endpoint is opened a second time as a capture
    /// client with AUDCLNT_STREAMFLAGS_LOOPBACK.
    pub fn capture_loop(analyzer: &mut Analyzer, analysis: &mut sonor_core::params::SonorAnalysis) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator = match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("sonor-listener: no audio stack: {:?}", e);
                    CoUninitialize();
                    return;
                }
            };
            let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("sonor-listener: no default output device: {:?}", e);
                    CoUninitialize();
                    return;
                }
            };
            let client: IAudioClient = match device.Activate(CLSCTX_ALL, None) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("sonor-listener: cannot activate the audio client: {:?}", e);
                    CoUninitialize();
                    return;
                }
            };
            let format = match client.GetMixFormat() {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("sonor-listener: cannot read the mix format: {:?}", e);
                    CoUninitialize();
                    return;
                }
            };
            let channels = (*format).nChannels as usize;
            let sample_rate = (*format).nSamplesPerSec;
            let bits = (*format).wBitsPerSample;
            let block_align = (*format).nBlockAlign as usize;
            let is_float = (*format).wFormatTag == 3;
            eprintln!(
                "sonor-listener: loopback on the default output — {} ch @ {} Hz, {} bit {}",
                channels,
                sample_rate,
                bits,
                if is_float { "float" } else { "pcm" }
            );

            let buffer_frames = ((sample_rate / 10).max(1024)) as i64;
            if client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    buffer_frames,
                    0i64,
                    format,
                    None,
                )
                .is_err()
            {
                eprintln!("sonor-listener: cannot initialise the capture client");
                CoUninitialize();
                return;
            }
            let capture: IAudioCaptureClient = match client.GetService() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("sonor-listener: no capture service: {:?}", e);
                    CoUninitialize();
                    return;
                }
            };
            if client.Start().is_err() {
                eprintln!("sonor-listener: cannot start the capture client");
                CoUninitialize();
                return;
            }

            let mut pending: Vec<f32> = Vec::with_capacity(FRAME * 8);
            loop {
                let mut packet = 0u32;
                if capture.GetNextPacketSize().is_ok() {
                    packet = capture.GetNextPacketSize().unwrap_or(0);
                }
                if packet == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                    continue;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                if !silent && !data.is_null() {
                    let total = frames as usize * block_align;
                    if is_float {
                        let slice = std::slice::from_raw_parts(data as *const f32, total / 4);
                        push_mono(&mut pending, slice, channels);
                    } else {
                        let slice = std::slice::from_raw_parts(data as *const i16, total / 2);
                        let scaled: Vec<f32> = slice.iter().map(|s| *s as f32 / 32768.0).collect();
                        push_mono(&mut pending, &scaled, channels);
                    }
                }
                let _ = capture.ReleaseBuffer(frames);
                while pending.len() >= FRAME {
                    analyzer.analyze(&pending[..FRAME], sample_rate as f32, HOP as f32, analysis);
                    emit(analysis);
                    pending.drain(0..HOP);
                }
            }
            let _ = client.Stop();
            CoUninitialize();
        }
    }

    fn push_mono(dst: &mut Vec<f32>, interleaved: &[f32], channels: usize) {
        if channels <= 1 {
            dst.extend_from_slice(interleaved);
            return;
        }
        let mut frame = [0.0f32; 8];
        let ch = channels.min(8);
        for chunk in interleaved.chunks(ch) {
            let n = chunk.len();
            for i in 0..n {
                frame[i] = chunk[i];
            }
            let sum: f32 = frame[..n].iter().sum();
            dst.push(sum / n as f32);
        }
    }

    pub fn list_devices() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Ok(enumerator) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            {
                if let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                    let count = collection.GetCount().unwrap_or(0);
                    for i in 0..count {
                        if let Ok(device) = collection.Item(i) {
                            if let Ok(id) = device.GetId() {
                                println!("{}", id.to_string().unwrap_or_default());
                            }
                        }
                    }
                }
            }
            CoUninitialize();
        }
    }
}
