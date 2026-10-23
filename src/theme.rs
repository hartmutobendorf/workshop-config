use clap::ValueEnum;
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Theme {
    #[default]
    Light,
    Dark,
    Paper,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub text: Color,
    pub background: Color,
    pub surface: Color,
    pub active: Color,
    pub border: Color,
    pub muted: Color,
    pub focus: Color,
    pub negative: Color,
    pub information: Color,
    pub brand: Color,
}

impl Palette {
    pub fn new(theme: Theme, color: bool) -> Self {
        if !color {
            return Self {
                text: Color::Reset,
                background: Color::Reset,
                surface: Color::Reset,
                active: Color::Reset,
                border: Color::Reset,
                muted: Color::Reset,
                focus: Color::Reset,
                negative: Color::Reset,
                information: Color::Reset,
                brand: Color::Reset,
            };
        }
        // DESIGN.md alpha colors are composited onto each theme's background.
        let mut palette = Self {
            text: Color::Rgb(0, 0, 0),
            background: Color::Rgb(255, 255, 255),
            surface: Color::Rgb(247, 247, 247),
            active: Color::Rgb(235, 235, 235),
            border: Color::Rgb(204, 204, 204),
            muted: Color::Rgb(102, 102, 102),
            focus: Color::Rgb(46, 150, 255),
            negative: Color::Rgb(199, 22, 43),
            information: Color::Rgb(36, 89, 143),
            brand: Color::Rgb(233, 84, 32),
        };
        match theme {
            Theme::Light => {}
            Theme::Paper => {
                palette.background = Color::Rgb(243, 243, 243);
                palette.active = Color::Rgb(224, 224, 224);
                palette.border = Color::Rgb(194, 194, 194);
                palette.muted = Color::Rgb(97, 97, 97);
            }
            Theme::Dark => {
                palette.text = Color::Rgb(255, 255, 255);
                palette.background = Color::Rgb(38, 38, 38);
                palette.surface = Color::Rgb(32, 32, 32);
                palette.active = Color::Rgb(55, 55, 55);
                palette.border = Color::Rgb(81, 81, 81);
                palette.muted = Color::Rgb(168, 168, 168);
                // Keep long feedback text readable; semantic labels also identify errors.
                palette.information = palette.text;
            }
        }
        palette
    }

    pub fn base(self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn selection(self) -> Style {
        self.base().bg(self.active).add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_design_tokens_and_supports_monochrome() {
        let light = Palette::new(Theme::Light, true);
        assert_eq!(light.background, Color::Rgb(255, 255, 255));
        assert_eq!(light.brand, Color::Rgb(233, 84, 32));
        assert_eq!(light.negative, Color::Rgb(199, 22, 43));
        assert_eq!(
            Palette::new(Theme::Dark, true).background,
            Color::Rgb(38, 38, 38)
        );
        assert_eq!(
            Palette::new(Theme::Paper, true).background,
            Color::Rgb(243, 243, 243)
        );
        let mono = Palette::new(Theme::Light, false);
        assert_eq!(mono.base().fg, Some(Color::Reset));
        assert_eq!(mono.selection().bg, Some(Color::Reset));
    }
}
