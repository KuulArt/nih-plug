//! REAPER's embedded FX views: the small live view REAPER can show for an FX inside the track
//! control panel (TCP) and the mixer strip (MCP). REAPER (6.24+) queries the VST3 controller for
//! `IReaperUIEmbedInterface` and sends it window messages; REAPER owns the window and hands the
//! plugin a 32-bit bitmap to paint into on its UI thread. A plugin opts in through
//! [`Vst3Plugin::reaper_embed()`][crate::prelude::Vst3Plugin::reaper_embed()]; without it the
//! interface answers that embedding is not supported, so nothing changes for other plugins.
//!
//! The message numbers and structs follow `reaper_plugin_fx_embed.h` from the REAPER SDK
//! (<https://github.com/justinfrankel/reaper-sdk>, zlib-style licence, see
//! `src/wrapper/vst3/reaper_embed.rs`). There is no CLAP equivalent yet.

use crate::prelude::GuiContext;

/// The raw message numbers (`REAPER_FXEMBED_WM_*`), aliases of win32's `WM_*`.
pub mod msg {
    pub const IS_SUPPORTED: i32 = 0x0000;
    pub const CREATE: i32 = 0x0001;
    pub const DESTROY: i32 = 0x0002;
    pub const PAINT: i32 = 0x000F;
    pub const SETCURSOR: i32 = 0x0020;
    pub const GETMINMAXINFO: i32 = 0x0024;
    pub const MOUSEMOVE: i32 = 0x0200;
    pub const LBUTTONDOWN: i32 = 0x0201;
    pub const LBUTTONUP: i32 = 0x0202;
    pub const LBUTTONDBLCLK: i32 = 0x0203;
    pub const RBUTTONDOWN: i32 = 0x0204;
    pub const RBUTTONUP: i32 = 0x0205;
    pub const RBUTTONDBLCLK: i32 = 0x0206;
    pub const MOUSEWHEEL: i32 = 0x020A;

    /// `REAPER_FXEMBED_RETNOTIFY_HANDLED`.
    pub const RET_HANDLED: isize = 0x0000001;
    /// `REAPER_FXEMBED_RETNOTIFY_INVALIDATE`: a non-optional redraw follows soon.
    pub const RET_INVALIDATE: isize = 0x1000000;

    /// `REAPER_FXEMBED_DRAWINFO_FLAG_*`.
    pub const FLAG_PAINT_OPTIONAL: i32 = 1;
    pub const FLAG_LBUTTON_CAPTURED: i32 = 0x10000;
    pub const FLAG_RBUTTON_CAPTURED: i32 = 0x20000;

    /// `REAPER_FXEMBED_EXT_GET_ADVISORY_SCALING`.
    pub const EXT_GET_ADVISORY_SCALING: i32 = 0x2003;
}

/// `REAPER_FXEMBED_DrawInfo`, as REAPER passes it.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawDrawInfo {
    pub context: i32,
    pub dpi: i32,
    pub mousewheel_amt: i32,
    pub _res2: f64,
    pub width: i32,
    pub height: i32,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub flags: i32,
    pub _res3: i32,
    pub spare: [*mut std::ffi::c_void; 6],
}

/// `REAPER_FXEMBED_SizeHints`, as REAPER passes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RawSizeHints {
    pub preferred_aspect: i32,
    pub minimum_aspect: i32,
    pub _res1: i32,
    pub _res2: i32,
    pub _res3: i32,
    pub _res4: i32,
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: i32,
    pub max_height: i32,
}

/// Where REAPER shows the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedPlace {
    /// REAPER 6.23 and earlier don't say.
    Unknown,
    /// The track control panel.
    Tcp,
    /// The mixer strip.
    Mcp,
}

/// What REAPER says about the view with a paint or a mouse message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbedInfo {
    pub place: EmbedPlace,
    /// REAPER's DPI scale (1.0 = 100 %), when it says.
    pub dpi: Option<f32>,
    /// Wheel amount of a [`EmbedMouse::Wheel`]: 120 is one step, a trackpad sends less.
    pub wheel: i32,
    /// The view's size, pixels.
    pub width: i32,
    pub height: i32,
    /// The mouse, pixels from the view's top left.
    pub mouse_x: i32,
    pub mouse_y: i32,
    /// A paint REAPER may skip: return `false` from [`ReaperEmbed::paint()`] if nothing moved.
    pub paint_optional: bool,
    pub left_captured: bool,
    pub right_captured: bool,
}

impl EmbedInfo {
    pub fn from_raw(raw: &RawDrawInfo) -> Self {
        EmbedInfo {
            place: match raw.context {
                1 => EmbedPlace::Tcp,
                2 => EmbedPlace::Mcp,
                _ => EmbedPlace::Unknown,
            },
            dpi: (raw.dpi > 0).then(|| raw.dpi as f32 / 256.0),
            wheel: raw.mousewheel_amt,
            width: raw.width,
            height: raw.height,
            mouse_x: raw.mouse_x,
            mouse_y: raw.mouse_y,
            paint_optional: raw.flags & msg::FLAG_PAINT_OPTIONAL != 0,
            left_captured: raw.flags & msg::FLAG_LBUTTON_CAPTURED != 0,
            right_captured: raw.flags & msg::FLAG_RBUTTON_CAPTURED != 0,
        }
    }
}

/// The mouse messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedMouse {
    /// Also sent once when the mouse leaves the view (uncaptured).
    Move,
    LeftDown,
    LeftUp,
    LeftDouble,
    RightDown,
    RightUp,
    RightDouble,
    Wheel,
}

impl EmbedMouse {
    pub fn from_msg(m: i32) -> Option<Self> {
        Some(match m {
            msg::MOUSEMOVE => EmbedMouse::Move,
            msg::LBUTTONDOWN => EmbedMouse::LeftDown,
            msg::LBUTTONUP => EmbedMouse::LeftUp,
            msg::LBUTTONDBLCLK => EmbedMouse::LeftDouble,
            msg::RBUTTONDOWN => EmbedMouse::RightDown,
            msg::RBUTTONUP => EmbedMouse::RightUp,
            msg::RBUTTONDBLCLK => EmbedMouse::RightDouble,
            msg::MOUSEWHEEL => EmbedMouse::Wheel,
            _ => return None,
        })
    }
}

/// The answer to a mouse message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedReply {
    /// Not the plugin's: REAPER does what it does with it (returns 0).
    Pass,
    /// The plugin took it.
    Handled,
    /// The plugin took it and the view must be redrawn.
    HandledRedraw,
}

impl EmbedReply {
    pub fn raw(self) -> isize {
        match self {
            EmbedReply::Pass => 0,
            EmbedReply::Handled => msg::RET_HANDLED,
            EmbedReply::HandledRedraw => msg::RET_HANDLED | msg::RET_INVALIDATE,
        }
    }
}

/// Size hints (`REAPER_FXEMBED_WM_GETMINMAXINFO`); REAPER may still pick any size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmbedSizeHints {
    /// Width over height.
    pub preferred_aspect: f32,
    pub minimum_aspect: f32,
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: i32,
    pub max_height: i32,
}

impl EmbedSizeHints {
    pub fn write(&self, raw: &mut RawSizeHints) {
        // 16.16 fixed point
        raw.preferred_aspect = (self.preferred_aspect * 65536.0).round() as i32;
        raw.minimum_aspect = (self.minimum_aspect * 65536.0).round() as i32;
        raw.min_width = self.min_width;
        raw.min_height = self.min_height;
        raw.max_width = self.max_width;
        raw.max_height = self.max_height;
    }
}

/// REAPER's bitmap for one paint: `height` rows of `row_span` pixels, each `0xAARRGGBB` in a
/// native-endian `u32` (BGRA in memory on little-endian machines), row 0 at the top unless
/// `flipped`.
pub struct EmbedCanvas<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    /// Pixels per row, including any past `width`.
    pub row_span: usize,
    /// Rows run bottom to top.
    pub flipped: bool,
    /// REAPER's advisory UI scale for this bitmap (1.0 = unscaled, 2.0 = 200 %), when it gives one.
    pub scale: Option<f32>,
}

impl EmbedCanvas<'_> {
    /// Row `y` counted from the top, `width` pixels.
    pub fn row_mut(&mut self, y: usize) -> &mut [u32] {
        let r = if self.flipped { self.height - 1 - y } else { y };
        let start = r * self.row_span;
        &mut self.pixels[start..start + self.width]
    }
}

/// A plugin's embedded view. Every call comes from REAPER's UI thread; `context` writes params as
/// the editor's [`GuiContext`] does (`beginEdit` / `performEdit` / `endEdit`).
pub trait ReaperEmbed: Send {
    /// `None` leaves REAPER's own size.
    fn size_hints(&self) -> Option<EmbedSizeHints> {
        None
    }

    /// Embedding began.
    fn created(&mut self, _context: &dyn GuiContext) {}

    /// Embedding ended: end any gesture still open.
    fn destroyed(&mut self, _context: &dyn GuiContext) {}

    /// Paint the view into `canvas`. Return `false` only when `info.paint_optional` and nothing
    /// changed since the last paint (the canvas is left as it is).
    fn paint(
        &mut self,
        canvas: &mut EmbedCanvas<'_>,
        info: &EmbedInfo,
        context: &dyn GuiContext,
    ) -> bool;

    /// A mouse message. [`EmbedReply::Pass`] leaves it to REAPER.
    fn mouse(
        &mut self,
        _event: EmbedMouse,
        _info: &EmbedInfo,
        _context: &dyn GuiContext,
    ) -> EmbedReply {
        EmbedReply::Pass
    }
}
