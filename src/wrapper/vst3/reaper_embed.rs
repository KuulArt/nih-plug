//! REAPER's `IReaperUIEmbedInterface` VST3 extension and its bitmap, `REAPER_FXEMBED_IBitmap`.
//! These are not part of the VST3 SDK or `vst3-sys`. Ported from the REAPER SDK's
//! `reaper_plugin_fx_embed.h` and `reaper_vst3_interfaces.h`
//! (<https://github.com/justinfrankel/reaper-sdk>), altered (translated to Rust), distributed
//! under their licence:
//!
//! > This software is provided 'as-is', without any express or implied warranty. In no event will
//! > the authors be held liable for any damages arising from the use of this software.
//! >
//! > Permission is granted to anyone to use this software for any purpose, including commercial
//! > applications, and to alter it and redistribute it freely, subject to the following
//! > restrictions:
//! >
//! > 1. The origin of this software must not be misrepresented; you must not claim that you wrote
//! >    the original software. If you use this software in a product, an acknowledgment in the
//! >    product documentation would be appreciated but is not required.
//! > 2. Altered source versions must be plainly marked as such, and must not be misrepresented as
//! >    being the original software.
//! > 3. This notice may not be removed or altered from any source distribution.

use std::ffi::c_void;

use parking_lot::Mutex;
use vst3_com::interfaces::iunknown::IUnknown;
use vst3_sys::com_interface;

use crate::prelude::GuiContext;
use crate::reaper_embed::{
    msg, EmbedCanvas, EmbedInfo, EmbedMouse, RawDrawInfo, RawSizeHints, ReaperEmbed,
};

// Alias needed for the `com_interface` macro
use vst3_sys as vst3_com;

/// Queried by REAPER 6.24+ from the plugin's `IEditController`.
#[com_interface("049BF9E7-BC74-EAD0-C410-1E867F725981")]
pub trait IReaperUIEmbedInterface: IUnknown {
    /// `msg` is a `REAPER_FXEMBED_WM_*`; `parm2` and `parm3` depend on it.
    unsafe fn embed_message(&self, msg: i32, parm2: isize, parm3: isize) -> isize;
}

/// `REAPER_FXEMBED_IBitmap` (an alias of WDL's `LICE_IBitmap`): a C++ class, called through its
/// vtable. A virtual destructor takes two slots in the Itanium ABI (macOS, Linux) and one in MSVC's.
#[repr(C)]
struct BitmapVtbl {
    #[cfg(not(target_os = "windows"))]
    _dtor: [usize; 2],
    #[cfg(target_os = "windows")]
    _dtor: [usize; 1],
    get_bits: unsafe extern "C" fn(*mut c_void) -> *mut u32,
    get_width: unsafe extern "C" fn(*mut c_void) -> i32,
    get_height: unsafe extern "C" fn(*mut c_void) -> i32,
    get_row_span: unsafe extern "C" fn(*mut c_void) -> i32,
    is_flipped: unsafe extern "C" fn(*mut c_void) -> bool,
    _resize: usize,
    _get_dc: usize,
    extended: unsafe extern "C" fn(*mut c_void, i32, *mut c_void) -> isize,
}

/// The canvas over REAPER's bitmap at `bitmap`, if it has pixels.
///
/// # Safety
///
/// `bitmap` must be null or REAPER's live `REAPER_FXEMBED_IBitmap*` for this paint.
unsafe fn canvas_of<'a>(bitmap: *mut c_void) -> Option<EmbedCanvas<'a>> {
    if bitmap.is_null() {
        return None;
    }
    let vtbl = &**(bitmap as *const *const BitmapVtbl);
    let bits = (vtbl.get_bits)(bitmap);
    let (w, h, span) = (
        (vtbl.get_width)(bitmap),
        (vtbl.get_height)(bitmap),
        (vtbl.get_row_span)(bitmap),
    );
    if bits.is_null() || w <= 0 || h <= 0 || span < w {
        return None;
    }
    let scale = (vtbl.extended)(bitmap, msg::EXT_GET_ADVISORY_SCALING, std::ptr::null_mut());
    Some(EmbedCanvas {
        pixels: std::slice::from_raw_parts_mut(bits, span as usize * h as usize),
        width: w as usize,
        height: h as usize,
        row_span: span as usize,
        flipped: (vtbl.is_flipped)(bitmap),
        scale: (scale > 0).then(|| scale as f32 / 256.0),
    })
}

/// Answers one message for `embed` (`None`: the plugin has no embedded view).
///
/// # Safety
///
/// `parm2` and `parm3` must be what REAPER passes with `m`.
pub(crate) unsafe fn dispatch(
    embed: Option<&Mutex<Box<dyn ReaperEmbed>>>,
    context: &dyn GuiContext,
    m: i32,
    parm2: isize,
    parm3: isize,
) -> isize {
    let Some(embed) = embed else {
        return 0;
    };
    if m == msg::IS_SUPPORTED {
        return 1;
    }
    // REAPER's UI thread is the only caller; a call re-entered from a param write is dropped
    let Some(mut embed) = embed.try_lock() else {
        return 0;
    };
    let info = || (parm3 != 0).then(|| EmbedInfo::from_raw(&*(parm3 as *const RawDrawInfo)));
    match m {
        msg::CREATE => {
            embed.created(context);
            0
        }
        msg::DESTROY => {
            embed.destroyed(context);
            0
        }
        msg::GETMINMAXINFO => match (embed.size_hints(), parm3 != 0) {
            (Some(h), true) => {
                h.write(&mut *(parm3 as *mut RawSizeHints));
                1
            }
            _ => 0,
        },
        msg::PAINT => match (canvas_of(parm2 as *mut c_void), info()) {
            (Some(mut canvas), Some(info)) => embed.paint(&mut canvas, &info, context) as isize,
            _ => 0,
        },
        _ => match (EmbedMouse::from_msg(m), info()) {
            (Some(event), Some(info)) => embed.mouse(event, &info, context).raw(),
            _ => 0,
        },
    }
}
