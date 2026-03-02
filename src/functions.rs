use ratatui::style::Color;
use ratatui::style::Color::*;

///Returns a random non-monochrome [`Color`]
pub(crate) fn random_color() -> Color {
    let colors = [
        Red,
        Green,
        Yellow,
        Blue,
        Magenta,
        Cyan,
        LightRed,
        LightGreen,
        LightYellow,
        LightBlue,
        LightMagenta,
        LightCyan,
    ];

    fastrand::choice(colors).unwrap()
}
