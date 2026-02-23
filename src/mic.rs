/// Toggle the default audio capture (microphone) mute state.
/// Uses the Windows Core Audio API — no third-party dependencies.
/// Profile-agnostic: called directly from the input loop on any profile.

use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::{eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};

pub fn toggle_mute() {
    unsafe {
        // Initialize COM on this thread (no-op if already done).
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

        log::info!("mic: {}", if muted { "unmuted" } else { "muted" });
    }
}
