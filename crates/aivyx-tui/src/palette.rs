//! The Aivyx terminal palette — Phase 185.
//!
//! The canonical brand colors for every TUI surface, lifted from the
//! "Command Center" GUI design tokens and translated to truecolor
//! ANSI (`Color::Rgb`). One source of truth so the chat surface
//! ([`crate::render`]), the design-study examples, and any future
//! state panels stay visually identical.
//!
//! Truecolor note: these are 24-bit RGB. Modern terminals render them
//! exactly; a 256-color terminal approximates to the nearest cube
//! entry, which keeps the amber/near-black identity recognizable. The
//! provenance prefixes (`❯`, `→`, `⚑`, …) carry meaning even with no
//! color at all.

use ratatui::style::{Color, Modifier, Style};

/// Near-black canvas (`#131319`).
pub const BG: Color = Color::Rgb(0x13, 0x13, 0x19);
/// Slightly-lighter panel fill (`#1c1b22`).
pub const PANEL: Color = Color::Rgb(0x1c, 0x1b, 0x22);
/// Darkest fill, for the status/footer bar (`#0d0d12`).
pub const STATUS_BG: Color = Color::Rgb(0x0d, 0x0d, 0x12);
/// Faint panel border (`#35343b`).
pub const BORDER: Color = Color::Rgb(0x35, 0x34, 0x3b);

/// Primary — amber (`#ffb77d`): operator input, gates, accents.
pub const AMBER: Color = Color::Rgb(0xff, 0xb7, 0x7d);
/// Secondary — lavender (`#ccc1e6`): provider/status accents.
pub const LAV: Color = Color::Rgb(0xcc, 0xc1, 0xe6);
/// Default foreground — off-white (`#e5e1eb`): agent prose.
pub const FG: Color = Color::Rgb(0xe5, 0xe1, 0xeb);
/// Dim slate (`#8a8695`): tool breadcrumbs, labels, help text.
pub const DIM: Color = Color::Rgb(0x8a, 0x86, 0x95);
/// Fainter slate (`#5a5a68`): separators, deep background detail.
pub const DIMMER: Color = Color::Rgb(0x5a, 0x5a, 0x68);
/// Healthy / success — emerald (`#34d399`).
pub const OK: Color = Color::Rgb(0x34, 0xd3, 0x99);
/// Error / over-limit — soft red (`#ffb4ab`).
pub const ERR: Color = Color::Rgb(0xff, 0xb4, 0xab);

/// `fg` only.
pub fn fg(c: Color) -> Style {
    Style::default().fg(c)
}

/// `fg` + bold.
pub fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}
