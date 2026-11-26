//! Colours, stored as 8-bit red, green, blue and alpha, written `#RRGGBBAA`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Error raised when a colour written as text cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("\"{text}\" is not a colour: expected #RRGGBB or #RRGGBBAA, for example #FF8800")]
pub struct ColorParseError {
    /// The rejected text.
    pub text: String,
}

/// An sRGB colour with transparency. `alpha` 255 is fully opaque, 0 fully transparent.
///
/// Serialized as text, `"#RRGGBBAA"`, so project files stay readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    /// Red, 0 to 255.
    pub red: u8,
    /// Green, 0 to 255.
    pub green: u8,
    /// Blue, 0 to 255.
    pub blue: u8,
    /// Opacity, 0 (transparent) to 255 (opaque).
    pub alpha: u8,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Self = Self::from_rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Self = Self::from_rgb(255, 255, 255);

    /// An opaque colour.
    pub const fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 255,
        }
    }

    /// Reads `#RRGGBB` or `#RRGGBBAA` (the `#` is optional, letters in any case).
    ///
    /// # Errors
    ///
    /// Fails when the text is not 6 or 8 hexadecimal digits.
    ///
    /// # Example
    ///
    /// ```
    /// use kadr_core::color::Color;
    ///
    /// assert_eq!(Color::from_hex("#ff8800").unwrap(), Color::from_rgb(255, 136, 0));
    /// ```
    pub fn from_hex(text: &str) -> Result<Self, ColorParseError> {
        let parse_error = || ColorParseError {
            text: text.to_string(),
        };
        let digits = text.strip_prefix('#').unwrap_or(text);
        if !digits.is_ascii() || !(digits.len() == 6 || digits.len() == 8) {
            return Err(parse_error());
        }
        let read_channel = |index: usize| {
            u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).map_err(|_| parse_error())
        };
        let alpha = if digits.len() == 8 {
            read_channel(3)?
        } else {
            255
        };
        Ok(Self {
            red: read_channel(0)?,
            green: read_channel(1)?,
            blue: read_channel(2)?,
            alpha,
        })
    }

    /// Writes the colour as `#RRGGBBAA` in capital letters.
    pub fn to_hex(self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            self.red, self.green, self.blue, self.alpha
        )
    }
}

impl fmt::Display for Color {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl TryFrom<String> for Color {
    type Error = ColorParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::from_hex(&text)
    }
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        color.to_hex()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_six_and_eight_digit_colours() {
        assert_eq!(Color::from_hex("000000").unwrap(), Color::BLACK);
        assert_eq!(
            Color::from_hex("#11223344").unwrap(),
            Color {
                red: 0x11,
                green: 0x22,
                blue: 0x33,
                alpha: 0x44
            }
        );
    }

    #[test]
    fn rejects_malformed_colours() {
        for text in ["", "#12345", "#GG0000", "#1234567", "#éé0000"] {
            assert!(Color::from_hex(text).is_err(), "{text}");
        }
    }

    #[test]
    fn round_trips_through_text_and_json() {
        let orange = Color::from_rgb(255, 136, 0);
        assert_eq!(orange.to_hex(), "#FF8800FF");
        assert_eq!(Color::from_hex(&orange.to_hex()).unwrap(), orange);
        assert_eq!(serde_json::to_string(&orange).unwrap(), "\"#FF8800FF\"");
    }
}
