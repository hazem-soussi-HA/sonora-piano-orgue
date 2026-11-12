//! WASAPI loopback capture, hand-rolled.
//!
//! No `windows` crate: the COM interfaces are called straight through their
//! vtables, so the listener builds with **zero dependencies** — nothing to
//! download, no build scripts, nothing an antivirus might object to.

#![allow(non_snake_case, non_camel_case_types)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;

pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;
pub const COINIT_MULTITHREADED: u32 = 0x2;
pub const CLSCTX_ALL: u32 = 0x17;
pub const E_RENDER: u32 = 0;
pub const E_CONSOLE: u32 = 0;
pub const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
pub const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
pub const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WavFormatEx {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}

const MMDEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xBCDE0395,
    data2: 0xE52F,
    data3: 0x467C,
    data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
    data1: 0xA95664D2,
    data2: 0x9614,
    data3: 0x4F35,
    data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};
const IID_IAUDIO_CLIENT: Guid = Guid {
    data1: 0x1CB9AD4C,
    data2: 0xDBFA,
    data3: 0x4C32,
    data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
};
const IID_IAUDIO_CAPTURE_CLIENT: Guid = Guid {
    data1: 0xC8ADBD64,
    data2: 0xE71E,
    data3: 0x48A0,
    data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
};

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pvreserved: *mut c_void, dwcoinit: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoCreateInstance(
        rclsid: *const Guid,
        punkouter: *mut c_void,
        dwclscontext: u32,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
}

/// Read a COM object's vtable.
unsafe fn vtable<T>(obj: *mut c_void) -> *const T {
    let vt = *(obj as *const *const c_void);
    vt as *const T
}

#[repr(C)]
struct UnknownVtbl {
    _query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    _add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    _release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct DeviceEnumeratorVtbl {
    unknown: UnknownVtbl,
    enum_audio_endpoints: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT,
    get_default_audio_endpoint: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DeviceVtbl {
    unknown: UnknownVtbl,
    activate: unsafe extern "system" fn(*mut c_void, *const c_void, u32, *const c_void, *const Guid, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct AudioClientVtbl {
    unknown: UnknownVtbl,
    initialize: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, *const WavFormatEx, *const Guid) -> HRESULT,
    get_buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_stream_latency: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
    get_current_padding: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_mix_format: unsafe extern "system" fn(*mut c_void, *mut *const WavFormatEx) -> HRESULT,
    get_device_period: unsafe extern "system" fn(*mut c_void, *mut u64) -> HRESULT,
    start: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    stop: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    set_event_handle: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    get_service: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct CaptureClientVtbl {
    unknown: UnknownVtbl,
    get_buffer: unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32, *mut c_void) -> HRESULT,
    release_buffer: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_next_packet_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}

/// The default render endpoint opened as a loopback capture.
pub struct Loopback {
    enumerator: *mut c_void,
    device: *mut c_void,
    client: *mut c_void,
    capture: *mut c_void,
    pub sample_rate: u32,
    pub channels: usize,
    pub float_format: bool,
    pub bits: u16,
    block_align: usize,
}

impl Loopback {
    /// Open the default render endpoint for loopback capture.
    pub fn open() -> Result<Loopback, String> {
        unsafe {
            CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);
            eprintln!("[wasapi] com initialised");
            eprintln!("[wasapi] creating the enumerator");
            let mut enumerator: *mut c_void = std::ptr::null_mut();
            let hr = CoCreateInstance(
                &MMDEVICE_ENUMERATOR,
                std::ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IMM_DEVICE_ENUMERATOR,
                &mut enumerator,
            );
            if hr != S_OK || enumerator.is_null() {
                return Err(format!("no audio stack (HRESULT 0x{:08x})", hr));
            }
            eprintln!("[wasapi] asking for the default endpoint");
            let mut device: *mut c_void = std::ptr::null_mut();
            let vt = vtable::<DeviceEnumeratorVtbl>(enumerator);
            let hr = ((*vt).get_default_audio_endpoint)(enumerator, E_RENDER, E_CONSOLE, &mut device);
            if hr != S_OK || device.is_null() {
                return Err(format!("no default output device (HRESULT 0x{:08x})", hr));
            }
            eprintln!("[wasapi] activating the client");
            let mut client: *mut c_void = std::ptr::null_mut();
            let dvt = vtable::<DeviceVtbl>(device);
            let hr = ((*dvt).activate)(
                device,
                std::ptr::null(),
                CLSCTX_ALL,
                std::ptr::null(),
                &IID_IAUDIO_CLIENT,
                &mut client,
            );
            if hr != S_OK || client.is_null() {
                return Err(format!("cannot activate the audio client (HRESULT 0x{:08x})", hr));
            }

            eprintln!("[wasapi] reading the mix format");
            let cvt = vtable::<AudioClientVtbl>(client);
            let mut format: *const WavFormatEx = std::ptr::null();
            let hr = ((*cvt).get_mix_format)(client, &mut format);
            if hr != S_OK || format.is_null() {
                return Err("cannot read the mix format".into());
            }
            let fmt = *format;
            let (float_format, bits) = match fmt.wFormatTag {
                3 => (true, 32),
                1 => (false, fmt.wBitsPerSample),
                0xFFFE => {
                    /* WAVE_FORMAT_EXTENSIBLE: the real tag hides in the GUID */
                    let ext = format as *const u8;
                    let sub = *(ext.add(24) as *const u32);
                    if sub == 3 {
                        (true, 32)
                    } else {
                        (false, fmt.wBitsPerSample)
                    }
                }
                other => return Err(format!("unsupported mix format {}", other)),
            };

            eprintln!("[wasapi] initialising, format tag={} ch={} sr={} bits={}", fmt.wFormatTag, fmt.nChannels, fmt.nSamplesPerSec, fmt.wBitsPerSample);
            let buffer_frames = (fmt.nSamplesPerSec / 10).max(1024);
            let hr = ((*cvt).initialize)(
                client,
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                buffer_frames,
                0,
                &fmt,
                std::ptr::null(),
            );
            if hr != S_OK {
                return Err(format!("cannot initialise the capture client (HRESULT 0x{:08x})", hr));
            }
            eprintln!("[wasapi] getting the capture service");
            let mut capture: *mut c_void = std::ptr::null_mut();
            let hr = ((*cvt).get_service)(client, &IID_IAUDIO_CAPTURE_CLIENT, &mut capture);
            if hr != S_OK || capture.is_null() {
                return Err("no capture service".into());
            }
            let hr = ((*cvt).start)(client);
            if hr != S_OK {
                return Err("cannot start the capture".into());
            }
            Ok(Loopback {
                enumerator,
                device,
                client,
                capture,
                sample_rate: fmt.nSamplesPerSec,
                channels: fmt.nChannels.max(1) as usize,
                float_format,
                bits,
                block_align: fmt.nBlockAlign.max(1) as usize,
            })
        }
    }

    /// Pull the next block of samples. Returns mono f32 in [-1, 1].
    pub fn read(&mut self, out: &mut Vec<f32>) -> bool {
        unsafe {
            let cvt = vtable::<CaptureClientVtbl>(self.capture);
            let mut packet: u32 = 0;
            if ((*cvt).get_next_packet_size)(self.capture, &mut packet) != S_OK || packet == 0 {
                return false;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if ((*cvt).get_buffer)(self.capture, &mut data, &mut frames, &mut flags, std::ptr::null_mut())
                != S_OK
            {
                return false;
            }
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT != 0;
            let total = frames as usize * self.block_align;
            if !silent && !data.is_null() {
                if self.float_format {
                    let slice = std::slice::from_raw_parts(data as *const f32, total / 4);
                    self.push_mono(out, slice);
                } else if self.bits == 32 {
                    let slice = std::slice::from_raw_parts(data as *const i32, total / 4);
                    let scaled: Vec<f32> = slice.iter().map(|v| *v as f32 / 2147483648.0).collect();
                    self.push_mono(out, &scaled);
                } else {
                    let slice = std::slice::from_raw_parts(data as *const i16, total / 2);
                    let scaled: Vec<f32> = slice.iter().map(|v| *v as f32 / 32768.0).collect();
                    self.push_mono(out, &scaled);
                }
            }
            let _ = ((*cvt).release_buffer)(self.capture, frames);
            true
        }
    }

    fn push_mono(&self, out: &mut Vec<f32>, interleaved: &[f32]) {
        let ch = self.channels.min(8);
        if ch <= 1 {
            out.extend_from_slice(interleaved);
            return;
        }
        for chunk in interleaved.chunks(ch) {
            let sum: f32 = chunk.iter().sum();
            out.push(sum / chunk.len() as f32);
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        unsafe {
            if !self.capture.is_null() {
                let vt = vtable::<CaptureClientVtbl>(self.capture);
                let _ = ((*vt).unknown._release)(self.capture);
            }
            if !self.client.is_null() {
                let vt = vtable::<AudioClientVtbl>(self.client);
                let _ = ((*vt).stop)(self.client);
                let _ = ((*vt).unknown._release)(self.client);
            }
            if !self.device.is_null() {
                let vt = vtable::<DeviceVtbl>(self.device);
                let _ = ((*vt).unknown._release)(self.device);
            }
            if !self.enumerator.is_null() {
                let vt = vtable::<DeviceEnumeratorVtbl>(self.enumerator);
                let _ = ((*vt).unknown._release)(self.enumerator);
            }
            CoUninitialize();
        }
    }
}
