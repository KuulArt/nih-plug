//! Bindings for the draft `clap.gain-adjustment-metering` extension, see
//! <https://github.com/free-audio/clap/blob/main/include/clap/ext/draft/gain-adjustment-metering.h>.

use clap_sys::plugin::clap_plugin;
use std::ffi::CStr;

pub const CLAP_EXT_GAIN_ADJUSTMENT_METERING: &CStr = c"clap.gain-adjustment-metering/0";

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct clap_plugin_gain_adjustment_metering {
    /// Returns the current gain adjustment in dB. Negative values mean the plugin is applying gain
    /// reduction. Called on the audio thread.
    pub get: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> f64>,
}
