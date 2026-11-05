//! Colours used by the editor renderers, for dark and light mode.

use eframe::egui::Color32;

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub text: Color32,
    pub strong: Color32,
    pub heading: Color32,
    pub weak: Color32,
    pub code: Color32,
    pub code_bg: Color32,
    pub link: Color32,
    pub accent: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                text: Color32::from_rgb(0xD4, 0xD4, 0xD4),
                strong: Color32::WHITE,
                heading: Color32::from_rgb(0xF0, 0xF0, 0xF0),
                weak: Color32::from_rgb(0x70, 0x76, 0x80),
                code: Color32::from_rgb(0xE6, 0xC0, 0x7B),
                code_bg: Color32::from_rgb(0x2A, 0x2D, 0x33),
                link: Color32::from_rgb(0x6C, 0xB6, 0xFF),
                accent: Color32::from_rgb(0xFF, 0xA6, 0x57),
            }
        } else {
            Self {
                dark,
                text: Color32::from_rgb(0x24, 0x29, 0x2F),
                strong: Color32::BLACK,
                heading: Color32::from_rgb(0x10, 0x14, 0x18),
                weak: Color32::from_rgb(0x8C, 0x95, 0x9F),
                code: Color32::from_rgb(0x95, 0x38, 0x00),
                code_bg: Color32::from_rgb(0xEF, 0xF1, 0xF3),
                link: Color32::from_rgb(0x09, 0x69, 0xDA),
                accent: Color32::from_rgb(0xBC, 0x4C, 0x00),
            }
        }
    }
}
