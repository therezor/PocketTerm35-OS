//! The drawing and input half of the pt35-desktop shell.
//!
//! Everything is software-rendered into a shared-memory buffer: on a 640x480
//! panel a full repaint is a fraction of a millisecond, and avoiding GL keeps
//! both the binary and the resident set small. GTK would cost more RAM than the
//! rest of the session put together.
//!
//! The modules split cleanly in two:
//!   * portable — `canvas`, `font`, `list`, `keys`, `pixel`: pure logic and pixels,
//!     unit-tested on any host;
//!   * Linux-only — `layer`: the Wayland layer-shell surface that puts those
//!     pixels on screen.

pub mod canvas;
pub mod font;
pub mod icon;
pub mod keys;
pub mod list;
pub mod pixel;

#[cfg(target_os = "linux")]
pub mod layer;

/// When `main` began, for the first-frame time in the debug log. The menu is
/// a new process per open, so that time is what opening it costs.
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Call first thing in `main`.
pub fn mark_start() {
    STARTED.get_or_init(std::time::Instant::now);
}

#[cfg(target_os = "linux")]
pub(crate) fn since_start() -> Option<std::time::Duration> {
    STARTED.get().map(|start| start.elapsed())
}

pub use canvas::Canvas;
pub use font::Font;
pub use keys::{Key, Navigation};
pub use list::ListState;
