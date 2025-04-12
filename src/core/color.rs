// Update src/core/color.rs to include a toggle feature

use std::env;

pub struct Color;

impl Color {
    // Foreground colors
    pub const RESET: &'static str = "\x1b[0m";
    pub const BLACK: &'static str = "\x1b[30m";
    pub const RED: &'static str = "\x1b[31m";
    pub const GREEN: &'static str = "\x1b[32m";
    pub const YELLOW: &'static str = "\x1b[33m";
    pub const BLUE: &'static str = "\x1b[34m";
    pub const MAGENTA: &'static str = "\x1b[35m";
    pub const CYAN: &'static str = "\x1b[36m";
    pub const WHITE: &'static str = "\x1b[37m";

    // Background colors
    pub const BG_BLACK: &'static str = "\x1b[40m";
    pub const BG_RED: &'static str = "\x1b[41m";
    pub const BG_GREEN: &'static str = "\x1b[42m";
    pub const BG_YELLOW: &'static str = "\x1b[43m";
    pub const BG_BLUE: &'static str = "\x1b[44m";
    pub const BG_MAGENTA: &'static str = "\x1b[45m";
    pub const BG_CYAN: &'static str = "\x1b[46m";
    pub const BG_WHITE: &'static str = "\x1b[47m";

    // Styles
    pub const BOLD: &'static str = "\x1b[1m";
    pub const UNDERLINE: &'static str = "\x1b[4m";
    pub const REVERSED: &'static str = "\x1b[7m";

    // Check if colors should be enabled
    fn is_enabled() -> bool {
        // Check for color support
        if let Ok(color_value) = env::var("ASH_COLOR") {
            match color_value.as_str() {
                "always" => true,
                "never" => false,
                _ => Self::has_color_support(),
            }
        } else {
            // Default to auto-detection
            Self::has_color_support()
        }
    }

    // Detect if terminal supports colors
    fn has_color_support() -> bool {
        if let Ok(term) = env::var("TERM") {
            // Most terminals with color support have TERM with "color" or "256"
            if term.contains("color") || term.contains("256") {
                return true;
            }
        }
        
        // Check for COLORTERM environment variable
        if let Ok(colorterm) = env::var("COLORTERM") {
            if !colorterm.is_empty() {
                return true;
            }
        }
        
        // Default to enabled
        true
    }

    // Helper function to color text
    pub fn colorize(text: &str, color: &str) -> String {
        if Self::is_enabled() {
            format!("{}{}{}", color, text, Self::RESET)
        } else {
            text.to_string()
        }
    }

    // Helper for green text
    pub fn green(text: &str) -> String {
        Self::colorize(text, Self::GREEN)
    }

    // Helper for red text
    pub fn red(text: &str) -> String {
        Self::colorize(text, Self::RED)
    }

    // Helper for yellow text
    pub fn yellow(text: &str) -> String {
        Self::colorize(text, Self::YELLOW)
    }

    // Helper for blue text
    pub fn blue(text: &str) -> String {
        Self::colorize(text, Self::BLUE)
    }

    // Helper for magenta text
    pub fn magenta(text: &str) -> String {
        Self::colorize(text, Self::MAGENTA)
    }

    // Helper for cyan text
    pub fn cyan(text: &str) -> String {
        Self::colorize(text, Self::CYAN)
    }

    // Helper for bold text
    pub fn bold(text: &str) -> String {
        Self::colorize(text, Self::BOLD)
    }
}