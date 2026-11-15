//! The settings cloofy follows: the animation switch and the battery saver of the system.

use std::ffi::c_void;

use windows::Win32::{
    System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW},
};
use windows::core::BOOL;

use crate::settings::Settings;

pub fn current() -> Settings {
    let mut animations = BOOL(1);
    // SAFETY: `SPI_GETCLIENTAREAANIMATION` writes one `BOOL` into the pointer it is given.
    let animations = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut animations as *mut BOOL as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map(|()| animations.as_bool())
    .unwrap_or(true);
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: the call fills in the status it is given, whose size it knows.
    let saving = unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.SystemStatusFlag == 1;
    settings(animations, saving)
}

/// The two readings as the settings: without animations there is less motion, and while the
/// battery saver runs the system is saving energy.
fn settings(animations: bool, saving: bool) -> Settings {
    Settings { reduce_motion: !animations, low_power: saving }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_switches() {
        assert_eq!(settings(true, false), Settings::default(), "animations on, no battery saver");
        let hidden = settings(false, true);
        assert!(hidden.reduce_motion);
        assert!(hidden.low_power);
    }
}
