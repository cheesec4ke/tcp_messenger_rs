use ratatui::prelude::Line;
use ratatui::style::Color;
use ratatui::style::Color::*;

///(todo) this is awful and doesn't work that well, there's probably a much better way to do this
pub(crate) fn calculate_wraps(lines: &Vec<Line>, area_width: usize) -> usize {
    let mut wraps = 0;
    for msg in lines {
        let mut width = 0;
        for span in msg {
            let mut first = true;
            for part in span.content.split(' ') {
                if !first {
                    width += 1;
                    if width > area_width {
                        wraps += 1;
                        width = 1;
                    }
                }
                width += part.len();
                if width > area_width {
                    wraps += 1;
                    width = part.len();
                    if width > area_width {
                        wraps += width / area_width;
                    }
                    first = true;
                } else {
                    first = false;
                }
            }
        }
    }

    wraps
}

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
