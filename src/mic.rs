/// Toggle the default audio capture (microphone) mute state.
/// Uses the Windows Core Audio API — no third-party dependencies.
/// Profile-agnostic: called directly from the input loop on any profile.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::{eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};

/// Cached mute state — written by toggle_mute() and init(), read by the output loop.
pub static MIC_MUTED: AtomicBool = AtomicBool::new(false);

/// Query the current system mute state and prime MIC_MUTED.
/// Call once at startup (on a blocking thread) before the first output frame.
pub fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Some(muted) = query_muted() {
            MIC_MUTED.store(muted, Ordering::Relaxed);
            log::debug!("mic: initial state = {}", if muted { "muted" } else { "unmuted" });
        }
    }
}

fn query_muted() -> Option<bool> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
        let vol: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        Some(vol.GetMute().ok()?.as_bool())
    }
}

/// Toggle system mic mute and update MIC_MUTED.
pub fn toggle_mute() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            log::warn!("mic: CoCreateInstance(MMDeviceEnumerator) failed");
            return;
        };

        let Ok(device) = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) else {
            log::warn!("mic: no default microphone found");
            return;
        };

        let Ok(vol): Result<IAudioEndpointVolume, _> = device.Activate(CLSCTX_ALL, None) else {
            log::warn!("mic: Activate(IAudioEndpointVolume) failed");
            return;
        };

        let muted = vol.GetMute().unwrap_or(BOOL(0)).as_bool();
        if let Err(e) = vol.SetMute(!muted, std::ptr::null()) {
            log::warn!("mic: SetMute failed: {e}");
            return;
        }

        let new_state = !muted;
        MIC_MUTED.store(new_state, Ordering::Relaxed);
        log::info!("mic: {}", if new_state { "muted" } else { "unmuted" });
    }
}
