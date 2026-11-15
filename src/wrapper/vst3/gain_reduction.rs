//! PreSonus' `IGainReductionInfo` VST3 extension. These extensions are not part of the VST3 SDK or
//! `vst3-sys`. The original header can be found at
//! <https://git.ardour.org/ardour/ardour/src/branch/master/libs/vst3/pslextensions/ipslgainreduction.h>.

use vst3_com::interfaces::iunknown::IUnknown;
use vst3_sys::com_interface;

// Alias needed for the `com_interface` macro
use vst3_sys as vst3_com;

/// Lets the host query the plugin's current gain reduction so it can be displayed in the host's
/// UI. Studio One and REAPER (`GainReduction_dB`) poll this periodically while the plugin is
/// active.
#[com_interface("8E3C292C-9592-4F9D-B259-0B1E100E4198")]
pub trait IGainReductionInfo: IUnknown {
    /// The current gain reduction in decibels. This is either 0.0 (no gain reduction) or negative.
    unsafe fn get_gain_reduction_value_in_db(&self) -> f64;
}
