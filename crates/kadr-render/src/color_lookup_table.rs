//! Colour lookup tables (LUTs): a look from a `.cube` file.
//!
//! A LUT maps every colour to another. A **3D** LUT is a cube of N × N × N
//! colours: the input's red, green and blue are three coordinates inside it. A
//! **1D** LUT gives each channel its own curve of N values.
//!
//! # The `.cube` format
//!
//! The text format of Adobe's "Cube LUT Specification 1.0", also written by DaVinci
//! Resolve and most grading tools:
//!
//! - `TITLE "…"`, optional;
//! - `LUT_3D_SIZE N` (2 to [`MAXIMUM_3D_SIZE`]) or `LUT_1D_SIZE N` (2 to
//!   [`MAXIMUM_1D_SIZE`]);
//! - `DOMAIN_MIN r g b` and `DOMAIN_MAX r g b`, the input range (0 and 1 by
//!   default), or Resolve's `LUT_1D_INPUT_RANGE min max` / `LUT_3D_INPUT_RANGE min max`;
//! - then the table, one `r g b` line per entry, **red varying fastest**, then
//!   green, then blue;
//! - `#` starts a comment; blank lines, Windows line endings and a byte-order mark
//!   are fine; unknown keywords are ignored.
//!
//! A file holding both a 1D curve and a 3D table (Resolve's "shaper" LUTs) is
//! refused with a clear message.
//!
//! # How a colour goes through
//!
//! 1. Each channel is placed in the domain, `(c − min) / (max − min)`, clamped to 0–1.
//! 2. 3D: trilinear interpolation between the 8 nearest colours of the cube (red,
//!    then green, then blue); 1D: linear interpolation between the 2 nearest values
//!    of each channel's curve.
//! 3. The result is mixed with the input by the strength, then clamped to 0–1.
//!
//! The same arithmetic runs on the CPU ([`LayerColorLut::apply`]) and on the GPU
//! (`look_up_color` in `gpu/layers.wgsl`, which reads the table with
//! `textureLoad`), so both renderers give the same colours.
//!
//! The LUT works on the same colours as the colour adjustments, which it follows:
//! gamma-encoded, full-range R'G'B' from 0 to 1, after the YUV → RGB conversion,
//! never linearised. That is what Rec.709 LUTs expect.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// The largest 3D table read: 65 × 65 × 65 colours (4.4 MB on the GPU). Larger
/// tables are rare and weigh tens of megabytes; they are refused with a message
/// saying to export the LUT at 33 or 65.
pub const MAXIMUM_3D_SIZE: usize = 65;
/// The largest 1D table read (the format's own limit).
pub const MAXIMUM_1D_SIZE: usize = 65_536;
/// How many colour LUTs one layer can have; further ones are ignored (and
/// reported by the export). Each takes a texture on the GPU.
pub const MAXIMUM_COLOR_LUTS_PER_LAYER: usize = 1;

/// The smallest size of a table: its two ends.
const MINIMUM_SIZE: usize = 2;
/// Width of the texture holding a 1D table: longer tables wrap onto more rows.
/// 256 is the smallest 3D texture side every GPU supports.
const ONE_DIMENSIONAL_TEXTURE_WIDTH: usize = 256;

/// The two kinds of tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LookupTableShape {
    /// One curve per channel.
    OneDimensional,
    /// A cube of colours.
    ThreeDimensional,
}

/// A colour lookup table, read from a `.cube` file.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorLookupTable {
    title: Option<String>,
    shape: LookupTableShape,
    size: usize,
    domain_minimum: [f32; 3],
    domain_maximum: [f32; 3],
    /// 3D: entry `r + g·N + b·N²`; 1D: entry `i` (one value per channel).
    entries: Vec<[f32; 3]>,
    fingerprint: u64,
}

/// Why a `.cube` file cannot be read. Messages never quote the file's lines:
/// a project may name a file the user did not choose.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CubeFileError {
    /// A line could not be understood.
    #[error("line {line_number}: {problem}")]
    InvalidLine {
        /// The line, counted from 1.
        line_number: usize,
        /// What is wrong with it.
        problem: String,
    },
    /// The file never says how large its table is.
    #[error("the file gives no table size: a .cube file needs a LUT_3D_SIZE or a LUT_1D_SIZE line")]
    MissingSize,
    /// The table size is outside what Kadr reads.
    #[error(
        "the {shape} size is {size}: Kadr reads sizes from {MINIMUM_SIZE} to {maximum} \
         (export the LUT again at 33 or 65)"
    )]
    UnsupportedSize {
        /// "3D table" or "1D table".
        shape: &'static str,
        /// The size given.
        size: usize,
        /// The largest size read for that shape.
        maximum: usize,
    },
    /// The number of colours does not match the size.
    #[error("the file has {found} colours, but a {shape} of size {size} needs {expected}")]
    WrongEntryCount {
        /// "3D table" or "1D table".
        shape: &'static str,
        /// The size given.
        size: usize,
        /// How many colours that size needs.
        expected: usize,
        /// How many the file has.
        found: usize,
    },
    /// The file holds both a 1D curve and a 3D table.
    #[error(
        "the file holds both a 1D curve and a 3D table (a \"shaper\" LUT), which Kadr does not \
         read: export the LUT again without a shaper"
    )]
    ShaperLutNotSupported,
    /// The domain is empty or reversed.
    #[error("the domain is empty: DOMAIN_MAX must be greater than DOMAIN_MIN for every channel")]
    InvalidDomain,
}

impl ColorLookupTable {
    /// Reads the text of a `.cube` file.
    ///
    /// # Errors
    ///
    /// See [`CubeFileError`].
    pub fn from_cube_text(text: &str) -> Result<Self, CubeFileError> {
        let mut reading = CubeReading::default();
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            reading
                .read_line(line)
                .map_err(|problem| CubeFileError::InvalidLine {
                    line_number,
                    problem,
                })?;
        }
        reading.finish()
    }

    /// A table from its entries (red varying fastest for a 3D table), with the
    /// default domain 0 to 1. For tests and for building looks in code.
    ///
    /// # Errors
    ///
    /// When the size or the number of entries is wrong.
    pub fn from_entries(
        shape: LookupTableShape,
        size: usize,
        entries: Vec<[f32; 3]>,
    ) -> Result<Self, CubeFileError> {
        Self::new(None, shape, size, [0.0; 3], [1.0; 3], entries)
    }

    fn new(
        title: Option<String>,
        shape: LookupTableShape,
        size: usize,
        domain_minimum: [f32; 3],
        domain_maximum: [f32; 3],
        entries: Vec<[f32; 3]>,
    ) -> Result<Self, CubeFileError> {
        let (shape_name, maximum, expected) = match shape {
            LookupTableShape::ThreeDimensional => {
                ("3D table", MAXIMUM_3D_SIZE, size.saturating_pow(3))
            }
            LookupTableShape::OneDimensional => ("1D table", MAXIMUM_1D_SIZE, size),
        };
        if !(MINIMUM_SIZE..=maximum).contains(&size) {
            return Err(CubeFileError::UnsupportedSize {
                shape: shape_name,
                size,
                maximum,
            });
        }
        if entries.len() != expected {
            return Err(CubeFileError::WrongEntryCount {
                shape: shape_name,
                size,
                expected,
                found: entries.len(),
            });
        }
        let is_domain_valid = domain_minimum
            .iter()
            .zip(&domain_maximum)
            .all(|(minimum, maximum)| minimum < maximum);
        if !is_domain_valid {
            return Err(CubeFileError::InvalidDomain);
        }
        let fingerprint = fingerprint_of(shape, size, domain_minimum, domain_maximum, &entries);
        Ok(Self {
            title,
            shape,
            size,
            domain_minimum,
            domain_maximum,
            entries,
            fingerprint,
        })
    }

    /// The file's `TITLE`, if it has one.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// 1D or 3D.
    pub fn shape(&self) -> LookupTableShape {
        self.shape
    }

    /// N: the side of the cube, or the length of the curves.
    pub fn size(&self) -> usize {
        self.size
    }

    /// A number identifying the table's content: two tables with the same
    /// fingerprint are the same table (the GPU keeps one texture for both).
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// The colour the table gives for `rgb` (each channel from 0 to 1). Not
    /// clamped: tables may hold values outside 0–1.
    pub fn look_up(&self, rgb: [f32; 3]) -> [f32; 3] {
        let position = self.position_in_domain(rgb);
        match self.shape {
            LookupTableShape::ThreeDimensional => self.look_up_in_cube(position),
            LookupTableShape::OneDimensional => self.look_up_in_curves(position),
        }
    }

    /// Each channel placed in the domain, from 0 to 1.
    fn position_in_domain(&self, rgb: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|channel| {
            let span = self.domain_maximum[channel] - self.domain_minimum[channel];
            ((rgb[channel] - self.domain_minimum[channel]) / span).clamp(0.0, 1.0)
        })
    }

    fn look_up_in_cube(&self, position: [f32; 3]) -> [f32; 3] {
        let (lower, fraction) = self.grid_position(position);
        let size = self.size;
        let entry = |red: usize, green: usize, blue: usize| {
            self.entries
                [lower[0] + red + (lower[1] + green) * size + (lower[2] + blue) * size * size]
        };
        // Along red, then green, then blue: the same order as the shader.
        let along_red =
            |green, blue| mix(entry(0, green, blue), entry(1, green, blue), fraction[0]);
        let along_green = |blue| mix(along_red(0, blue), along_red(1, blue), fraction[1]);
        mix(along_green(0), along_green(1), fraction[2])
    }

    fn look_up_in_curves(&self, position: [f32; 3]) -> [f32; 3] {
        let (lower, fraction) = self.grid_position(position);
        std::array::from_fn(|channel| {
            let before = self.entries[lower[channel]][channel];
            let after = self.entries[lower[channel] + 1][channel];
            before * (1.0 - fraction[channel]) + after * fraction[channel]
        })
    }

    /// For each channel, the grid node below the position and how far past it
    /// the position is (0 to 1). The last node is reached with a fraction of 1.
    fn grid_position(&self, position: [f32; 3]) -> ([usize; 3], [f32; 3]) {
        let last_index = (self.size - 1) as f32;
        let mut lower = [0; 3];
        let mut fraction = [0.0; 3];
        for channel in 0..3 {
            let scaled = position[channel] * last_index;
            let node = scaled.floor().min(last_index - 1.0);
            lower[channel] = node as usize;
            fraction[channel] = scaled - node;
        }
        (lower, fraction)
    }

    /// Size of the GPU texture holding the table: N × N × N for a cube; the
    /// curves wrapped onto rows of at most 256 entries for 1D.
    pub(crate) fn texture_extent(&self) -> [u32; 3] {
        match self.shape {
            LookupTableShape::ThreeDimensional => [self.size as u32; 3],
            LookupTableShape::OneDimensional => {
                let width = self.size.min(ONE_DIMENSIONAL_TEXTURE_WIDTH);
                [width as u32, self.size.div_ceil(width) as u32, 1]
            }
        }
    }

    /// The texels of [`Self::texture_extent`], RGBA floats in texture order (x
    /// fastest, which is the file's order: red fastest). The unused end of the
    /// last row of a 1D table repeats its last entry.
    pub(crate) fn texture_texels(&self) -> Vec<f32> {
        let [width, height, depth] = self.texture_extent();
        let texel_count = (width * height * depth) as usize;
        let last_entry = self.entries.last().copied().unwrap_or_default();
        (0..texel_count)
            .flat_map(|index| {
                let [red, green, blue] = self.entries.get(index).copied().unwrap_or(last_entry);
                [red, green, blue, 1.0]
            })
            .collect()
    }

    /// The three rows the shader reads (see `gpu/layers.wgsl`):
    /// - x: 1 for 1D, 2 for 3D; y: N; z: strength; w: width of a 1D texture row;
    /// - the domain minimum;
    /// - 1 / (maximum − minimum) per channel.
    pub(crate) fn shader_rows(&self, strength_fraction: f32) -> [[f32; 4]; 3] {
        let kind = match self.shape {
            LookupTableShape::OneDimensional => 1.0,
            LookupTableShape::ThreeDimensional => 2.0,
        };
        let [row_width, _, _] = self.texture_extent();
        let [minimum_red, minimum_green, minimum_blue] = self.domain_minimum;
        let scale: [f32; 3] = std::array::from_fn(|channel| {
            1.0 / (self.domain_maximum[channel] - self.domain_minimum[channel])
        });
        [
            [kind, self.size as f32, strength_fraction, row_width as f32],
            [minimum_red, minimum_green, minimum_blue, 0.0],
            [scale[0], scale[1], scale[2], 0.0],
        ]
    }
}

/// What the shader reads when a layer has no LUT.
pub(crate) const NO_LUT_SHADER_ROWS: [[f32; 4]; 3] = [[0.0; 4]; 3];

/// A table applied to a layer, with its strength.
#[derive(Debug, Clone, Copy)]
pub struct LayerColorLut<'table> {
    /// The table.
    pub table: &'table ColorLookupTable,
    /// 0 keeps the layer's colours, 1 applies the table fully.
    pub strength_fraction: f32,
}

impl LayerColorLut<'_> {
    /// Whether the LUT changes anything.
    pub fn is_visible(&self) -> bool {
        self.strength_fraction > 0.0
    }

    /// `rgb` (from 0 to 1) through the table, mixed with itself by the
    /// strength, clamped to 0–1.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let looked_up = self.table.look_up(rgb);
        mix(rgb, looked_up, self.strength_fraction).map(|channel| channel.clamp(0.0, 1.0))
    }
}

/// `first · (1 − fraction) + second · fraction`, per channel: WGSL's `mix`.
fn mix(first: [f32; 3], second: [f32; 3], fraction: f32) -> [f32; 3] {
    std::array::from_fn(|channel| first[channel] * (1.0 - fraction) + second[channel] * fraction)
}

fn fingerprint_of(
    shape: LookupTableShape,
    size: usize,
    domain_minimum: [f32; 3],
    domain_maximum: [f32; 3],
    entries: &[[f32; 3]],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    shape.hash(&mut hasher);
    size.hash(&mut hasher);
    for value in domain_minimum
        .iter()
        .chain(&domain_maximum)
        .chain(entries.iter().flatten())
    {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// What has been read so far from a `.cube` file.
#[derive(Debug, Default)]
struct CubeReading {
    title: Option<String>,
    size_3d: Option<usize>,
    size_1d: Option<usize>,
    domain_minimum: Option<[f32; 3]>,
    domain_maximum: Option<[f32; 3]>,
    input_range: Option<(f32, f32)>,
    entries: Vec<[f32; 3]>,
}

impl CubeReading {
    fn read_line(&mut self, line: &str) -> Result<(), String> {
        let starts_with_letter = line.chars().next().is_some_and(char::is_alphabetic);
        if !starts_with_letter {
            self.entries.push(three_numbers(line)?);
            return Ok(());
        }
        let (keyword, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match keyword {
            "TITLE" => self.title = Some(rest.trim_matches('"').to_string()),
            "LUT_3D_SIZE" => self.size_3d = Some(one_size(rest)?),
            "LUT_1D_SIZE" => self.size_1d = Some(one_size(rest)?),
            "DOMAIN_MIN" => self.domain_minimum = Some(three_numbers(rest)?),
            "DOMAIN_MAX" => self.domain_maximum = Some(three_numbers(rest)?),
            "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                self.input_range = Some(two_numbers(rest)?)
            }
            // Other keywords (LUT_IN_VIDEO_RANGE…) change nothing Kadr does.
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<ColorLookupTable, CubeFileError> {
        let (shape, size) = match (self.size_3d, self.size_1d) {
            (Some(_), Some(_)) => return Err(CubeFileError::ShaperLutNotSupported),
            (Some(size), None) => (LookupTableShape::ThreeDimensional, size),
            (None, Some(size)) => (LookupTableShape::OneDimensional, size),
            (None, None) => return Err(CubeFileError::MissingSize),
        };
        let (range_minimum, range_maximum) = self.input_range.unwrap_or((0.0, 1.0));
        let domain_minimum = self.domain_minimum.unwrap_or([range_minimum; 3]);
        let domain_maximum = self.domain_maximum.unwrap_or([range_maximum; 3]);
        ColorLookupTable::new(
            self.title,
            shape,
            size,
            domain_minimum,
            domain_maximum,
            self.entries,
        )
    }
}

fn numbers_in(text: &str) -> Result<Vec<f32>, String> {
    text.split_whitespace()
        .map(|word| {
            word.parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| "a value is not a finite number".to_string())
        })
        .collect()
}

fn three_numbers(text: &str) -> Result<[f32; 3], String> {
    let numbers = numbers_in(text)?;
    <[f32; 3]>::try_from(numbers.as_slice()).map_err(|_| {
        format!(
            "expected three numbers (red, green, blue), found {}",
            numbers.len()
        )
    })
}

fn two_numbers(text: &str) -> Result<(f32, f32), String> {
    match numbers_in(text)?[..] {
        [minimum, maximum] => Ok((minimum, maximum)),
        ref other => Err(format!(
            "expected two numbers (minimum, maximum), found {}",
            other.len()
        )),
    }
}

fn one_size(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|_| "the size is not a whole number".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3D table of `size` computed from a function of the node's colour.
    fn cube_of(size: usize, colour_at: impl Fn([f32; 3]) -> [f32; 3]) -> ColorLookupTable {
        let last = (size - 1) as f32;
        let entries = (0..size * size * size)
            .map(|index| {
                let red = (index % size) as f32 / last;
                let green = (index / size % size) as f32 / last;
                let blue = (index / (size * size)) as f32 / last;
                colour_at([red, green, blue])
            })
            .collect();
        ColorLookupTable::from_entries(LookupTableShape::ThreeDimensional, size, entries).unwrap()
    }

    fn assert_close(actual: [f32; 3], expected: [f32; 3]) {
        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 1e-5,
                "{actual:?} instead of {expected:?}"
            );
        }
    }

    #[test]
    fn an_identity_cube_gives_the_colour_back() {
        for size in [2, 17, 33] {
            let table = cube_of(size, |colour| colour);
            for colour in [
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [0.2, 0.55, 0.93],
                [0.5, 0.0, 1.0],
            ] {
                assert_close(table.look_up(colour), colour);
            }
        }
    }

    #[test]
    fn a_linear_look_is_exact_and_the_axes_are_in_file_order() {
        // Red and blue swapped and inverted: catches a red-fastest mix-up.
        let table = cube_of(9, |[red, green, blue]| [1.0 - blue, green, 1.0 - red]);
        assert_close(table.look_up([0.1, 0.4, 0.7]), [0.3, 0.4, 0.9]);
        assert_close(table.look_up([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn the_domain_places_colours_in_the_table() {
        let text = "LUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n\
                    0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let table = ColorLookupTable::from_cube_text(text).unwrap();
        assert_close(table.look_up([1.0, 0.5, 0.0]), [0.5, 0.25, 0.0]);
        assert_close(table.look_up([4.0, -1.0, 0.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn one_dimensional_curves_shape_each_channel() {
        // Red squared roughly, green inverted, blue unchanged.
        let text = "TITLE \"Curves\"\nLUT_1D_SIZE 3\n0 1 0\n0.25 0.5 0.5\n1 0 1\n";
        let table = ColorLookupTable::from_cube_text(text).unwrap();
        assert_eq!(table.title(), Some("Curves"));
        assert_eq!(table.shape(), LookupTableShape::OneDimensional);
        assert_close(table.look_up([0.5, 0.5, 0.75]), [0.25, 0.5, 0.75]);
        assert_close(table.look_up([0.75, 1.0, 0.0]), [0.625, 0.0, 0.0]);
    }

    #[test]
    fn strength_mixes_with_the_input_and_the_result_is_clamped() {
        let inverting = cube_of(2, |[red, green, blue]| [1.0 - red, 1.0 - green, 1.0 - blue]);
        let half = LayerColorLut {
            table: &inverting,
            strength_fraction: 0.5,
        };
        assert_close(half.apply([0.2, 0.6, 1.0]), [0.5, 0.5, 0.5]);
        let nothing = LayerColorLut {
            table: &inverting,
            strength_fraction: 0.0,
        };
        assert!(!nothing.is_visible());
        assert_close(nothing.apply([0.2, 0.6, 1.0]), [0.2, 0.6, 1.0]);
        let too_bright = cube_of(2, |_| [1.5, -0.5, 0.5]);
        let full = LayerColorLut {
            table: &too_bright,
            strength_fraction: 1.0,
        };
        assert_close(full.apply([0.3, 0.3, 0.3]), [1.0, 0.0, 0.5]);
    }

    #[test]
    fn real_files_with_comments_crlf_and_a_byte_order_mark_are_read() {
        let text = "\u{feff}# Created by a grading tool\r\nTITLE \"Warm\"\r\nLUT_3D_SIZE 2\r\n\r\n\
                    0 0 0\r\n1 0 0 # red\r\n0 1 0\r\n1 1 0\r\n0 0 1\r\n1 0 1\r\n0 1 1\r\n1 1 1\r\n\
                    LUT_IN_VIDEO_RANGE\r\n";
        let table = ColorLookupTable::from_cube_text(text).unwrap();
        assert_eq!((table.title(), table.size()), (Some("Warm"), 2));
        assert_close(table.look_up([0.25, 0.5, 0.75]), [0.25, 0.5, 0.75]);
        let ranged = "LUT_3D_INPUT_RANGE 0 2\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert_close(
            ColorLookupTable::from_cube_text(ranged)
                .unwrap()
                .look_up([1.0, 1.0, 1.0]),
            [0.5, 0.5, 0.5],
        );
    }

    #[test]
    fn broken_files_are_refused_with_the_line_or_the_reason() {
        let refusal = |text: &str| ColorLookupTable::from_cube_text(text).unwrap_err();
        assert_eq!(refusal("0 0 0\n"), CubeFileError::MissingSize);
        assert!(matches!(
            refusal("LUT_3D_SIZE 2\n0 0 0\n"),
            CubeFileError::WrongEntryCount {
                expected: 8,
                found: 1,
                ..
            }
        ));
        assert_eq!(
            refusal("LUT_3D_SIZE 2\n0 0\n").to_string(),
            "line 2: expected three numbers (red, green, blue), found 2"
        );
        assert_eq!(
            refusal("LUT_3D_SIZE 2\n0 zero 0\n").to_string(),
            "line 2: a value is not a finite number"
        );
        assert!(matches!(
            refusal("LUT_3D_SIZE 129\n"),
            CubeFileError::UnsupportedSize { size: 129, .. }
        ));
        assert!(matches!(
            refusal("LUT_1D_SIZE 1\n0 0 0\n"),
            CubeFileError::UnsupportedSize { .. }
        ));
        assert_eq!(
            refusal("LUT_1D_SIZE 2\nLUT_3D_SIZE 2\n"),
            CubeFileError::ShaperLutNotSupported
        );
        assert_eq!(
            refusal("LUT_1D_SIZE 2\nDOMAIN_MIN 1 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n"),
            CubeFileError::InvalidDomain
        );
    }

    #[test]
    fn equal_tables_share_a_fingerprint_and_different_ones_do_not() {
        let identity = cube_of(5, |colour| colour);
        assert_eq!(
            identity.fingerprint(),
            cube_of(5, |colour| colour).fingerprint()
        );
        assert_ne!(
            identity.fingerprint(),
            cube_of(5, |[red, green, blue]| [blue, green, red]).fingerprint()
        );
    }

    #[test]
    fn textures_hold_the_table_in_file_order() {
        let cube = cube_of(3, |colour| colour);
        assert_eq!(cube.texture_extent(), [3, 3, 3]);
        let texels = cube.texture_texels();
        assert_eq!(texels.len(), 27 * 4);
        assert_eq!(
            &texels[4..8],
            &[0.5, 0.0, 0.0, 1.0],
            "the second texel is red's next node"
        );
        let long_curve = ColorLookupTable::from_entries(
            LookupTableShape::OneDimensional,
            300,
            (0..300).map(|index| [index as f32 / 299.0; 3]).collect(),
        )
        .unwrap();
        assert_eq!(long_curve.texture_extent(), [256, 2, 1]);
        let texels = long_curve.texture_texels();
        assert_eq!(texels.len(), 512 * 4);
        assert_eq!(
            texels[511 * 4],
            1.0,
            "the end of the last row repeats the last entry"
        );
        assert_eq!(long_curve.shader_rows(0.5)[0], [1.0, 300.0, 0.5, 256.0]);
    }
}
