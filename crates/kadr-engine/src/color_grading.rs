//! Colour grading with LUT files (`.cube`): reading them from the disk.
//!
//! A clip's colour LUT effect only names a file (see `kadr_core::effect::ColorLut`).
//! The engine reads it here: [`load_color_lookup_table`] checks the file's size,
//! reads its text and parses it with `kadr-render`, which owns the format and
//! applies the table on the CPU and the GPU.
//!
//! Interfaces call [`load_color_lookup_table`] **before** adding a LUT to a clip,
//! so a wrong file is reported at once, and show its [`ColorTableSummary`]. While
//! rendering, the export and the preview keep the tables they read and read a file
//! again only when its modification time or length changes, so a look exported
//! again from a grading tool shows on the next preview frame. A file that has gone
//! missing or cannot be read is **skipped**: the clip is drawn without its look and
//! the export report says so. It never stops an export.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use kadr_core::effect::{ColorLut, EffectKind};
use kadr_core::project::Project;
pub use kadr_render::color_lookup_table::{
    ColorLookupTable, CubeFileError, LookupTableShape, MAXIMUM_COLOR_LUTS_PER_LAYER,
};

/// The largest `.cube` file read. A 65 × 65 × 65 table (the largest size read)
/// written with ten digits per number weighs about 11 MB; anything much larger is
/// not a LUT Kadr can use, and is refused before being read.
pub const MAXIMUM_CUBE_FILE_SIZE_IN_BYTES: u64 = 32 * 1024 * 1024;

/// Why a LUT file could not be used.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ColorTableError {
    /// The file could not be read (missing, locked, too large).
    #[error("could not read the colour LUT \"{}\": {reason}", file_path.display())]
    CannotRead {
        /// The file.
        file_path: PathBuf,
        /// What went wrong.
        reason: String,
    },
    /// The file was read but is not a `.cube` table Kadr understands.
    #[error("\"{}\" is not a colour LUT Kadr can read: {source}", file_path.display())]
    InvalidContent {
        /// The file.
        file_path: PathBuf,
        /// What is wrong with it (with the line number when it is one line).
        source: CubeFileError,
    },
}

impl ColorTableError {
    /// The file the error is about.
    pub fn file_path(&self) -> &Path {
        match self {
            Self::CannotRead { file_path, .. } | Self::InvalidContent { file_path, .. } => {
                file_path
            }
        }
    }
}

/// What interfaces show about a table: its title, its shape and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorTableSummary {
    /// The file's `TITLE`, if it has one.
    pub title: Option<String>,
    /// 1D (one curve per channel) or 3D (a cube of colours).
    pub shape: LookupTableShape,
    /// The side of the cube, or the length of the curves.
    pub size: usize,
}

impl ColorTableSummary {
    /// The summary of `table`.
    pub fn of(table: &ColorLookupTable) -> Self {
        Self {
            title: table.title().map(str::to_string),
            shape: table.shape(),
            size: table.size(),
        }
    }

    /// The size in words: "33 × 33 × 33" for a 3D table, "1D, 1024 values" for curves.
    pub fn size_in_words(&self) -> String {
        match self.shape {
            LookupTableShape::ThreeDimensional => {
                format!("{0} × {0} × {0}", self.size)
            }
            LookupTableShape::OneDimensional => format!("1D, {} values", self.size),
        }
    }
}

/// Reads and parses the `.cube` file at `file_path`.
///
/// Files larger than [`MAXIMUM_CUBE_FILE_SIZE_IN_BYTES`] are refused before being
/// read. Text that is not valid UTF-8 is read with replacement characters (a title
/// in another encoding does not matter; a broken number is reported by its line).
///
/// # Errors
///
/// [`ColorTableError::CannotRead`] when the file cannot be read or is too large,
/// [`ColorTableError::InvalidContent`] when it is not a table Kadr reads.
pub fn load_color_lookup_table(file_path: &Path) -> Result<Arc<ColorLookupTable>, ColorTableError> {
    let cannot_read = |reason: String| ColorTableError::CannotRead {
        file_path: file_path.to_path_buf(),
        reason,
    };
    let metadata = fs::metadata(file_path).map_err(|error| cannot_read(error.to_string()))?;
    if !metadata.is_file() {
        return Err(cannot_read("it is not a file".to_string()));
    }
    if metadata.len() > MAXIMUM_CUBE_FILE_SIZE_IN_BYTES {
        return Err(cannot_read(format!(
            "it weighs {} MB, more than any LUT Kadr reads ({} MB at most)",
            metadata.len().div_ceil(1024 * 1024),
            MAXIMUM_CUBE_FILE_SIZE_IN_BYTES / (1024 * 1024)
        )));
    }
    let bytes = fs::read(file_path).map_err(|error| cannot_read(error.to_string()))?;
    let text = String::from_utf8_lossy(&bytes);
    ColorLookupTable::from_cube_text(&text)
        .map(Arc::new)
        .map_err(|source| ColorTableError::InvalidContent {
            file_path: file_path.to_path_buf(),
            source,
        })
}

/// The LUT a clip's effects apply: the first enabled colour LUT, if it has some
/// strength. Further LUTs are ignored (the export reports them).
pub(crate) fn applied_color_lut_of(effects: &[kadr_core::effect::Effect]) -> Option<&ColorLut> {
    effects
        .iter()
        .filter(|effect| effect.is_enabled)
        .find_map(|effect| match &effect.kind {
            EffectKind::ColorLut(color_lut) => Some(color_lut),
            EffectKind::ColorAdjustment(_) | EffectKind::TrackedBlur(_) => None,
        })
        .filter(|color_lut| color_lut.strength_fraction > 0.0)
}

/// When a file was last written, and its length: a file whose stamp changed is
/// read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    length_in_bytes: u64,
}

impl FileStamp {
    /// The stamp of `file_path`, or `None` if the file cannot be seen.
    fn of(file_path: &Path) -> Option<Self> {
        let metadata = fs::metadata(file_path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            length_in_bytes: metadata.len(),
        })
    }
}

/// A file read once, or the reason it could not be.
#[derive(Debug)]
struct ReadTable {
    stamp: Option<FileStamp>,
    outcome: Result<Arc<ColorLookupTable>, ColorTableError>,
}

/// The tables read for one export or one preview, by file.
///
/// Failures are kept too, so a missing file is not looked for again on every
/// frame (only its stamp is checked). Not shared between threads: each export or
/// preview has its own.
#[derive(Debug, Default)]
pub(crate) struct ColorTableLibrary {
    tables: HashMap<PathBuf, ReadTable>,
}

impl ColorTableLibrary {
    /// The table of `file_path`, read again if the file changed since it was read.
    pub(crate) fn table_at(
        &mut self,
        file_path: &Path,
    ) -> Result<Arc<ColorLookupTable>, ColorTableError> {
        let stamp = FileStamp::of(file_path);
        if let Some(read_table) = self.tables.get(file_path)
            && read_table.stamp == stamp
        {
            return read_table.outcome.clone();
        }
        let outcome = load_color_lookup_table(file_path);
        self.tables.insert(
            file_path.to_path_buf(),
            ReadTable {
                stamp,
                outcome: outcome.clone(),
            },
        );
        outcome
    }

    /// Forgets the tables of files `project` no longer uses.
    pub(crate) fn forget_files_unused_by(&mut self, project: &Project) {
        if self.tables.is_empty() {
            return;
        }
        let used_files: HashSet<&Path> = project
            .timeline()
            .all_clips()
            .flat_map(|(_, clip)| clip.effects.iter())
            .filter_map(|effect| match &effect.kind {
                EffectKind::ColorLut(color_lut) => Some(color_lut.file_path.as_path()),
                EffectKind::ColorAdjustment(_) | EffectKind::TrackedBlur(_) => None,
            })
            .collect();
        self.tables
            .retain(|file_path, _| used_files.contains(file_path.as_path()));
    }

    /// How many files are kept, for tests.
    #[cfg(test)]
    fn kept_file_count(&self) -> usize {
        self.tables.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 × 2 × 2 table turning every colour into its opposite.
    const INVERTING_CUBE: &str = "TITLE \"Invert\"\nLUT_3D_SIZE 2\n\
        1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
    /// The same table without a title (another length).
    const UNTITLED_INVERTING_CUBE: &str = "LUT_3D_SIZE 2\n\
        1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";

    fn write_file(folder: &tempfile::TempDir, name: &str, text: &str) -> PathBuf {
        let file_path = folder.path().join(name);
        fs::write(&file_path, text).unwrap();
        file_path
    }

    #[test]
    fn a_cube_file_is_read_and_summarised() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = write_file(&folder, "invert.cube", INVERTING_CUBE);

        let table = load_color_lookup_table(&file_path).unwrap();
        let summary = ColorTableSummary::of(&table);

        assert_eq!(summary.title.as_deref(), Some("Invert"));
        assert_eq!(summary.shape, LookupTableShape::ThreeDimensional);
        assert_eq!(summary.size_in_words(), "2 × 2 × 2");
        assert_eq!(table.look_up([1.0, 0.0, 0.0]), [0.0, 1.0, 1.0]);
    }

    #[test]
    fn a_missing_file_names_its_path() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = folder.path().join("gone.cube");

        let error = load_color_lookup_table(&file_path).unwrap_err();

        assert!(matches!(error, ColorTableError::CannotRead { .. }));
        assert_eq!(error.file_path(), file_path);
        assert!(error.to_string().contains("gone.cube"), "{error}");
    }

    #[test]
    fn a_wrong_line_is_reported_by_its_number_without_quoting_it() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = write_file(
            &folder,
            "broken.cube",
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 secret\n",
        );

        let error = load_color_lookup_table(&file_path).unwrap_err();
        let message = error.to_string();

        assert!(matches!(error, ColorTableError::InvalidContent { .. }));
        assert!(message.contains("line 4"), "{message}");
        assert!(message.contains("broken.cube"), "{message}");
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn a_file_too_large_is_refused_before_being_read() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = folder.path().join("huge.cube");
        let file = fs::File::create(&file_path).unwrap();
        file.set_len(MAXIMUM_CUBE_FILE_SIZE_IN_BYTES + 1).unwrap();

        let error = load_color_lookup_table(&file_path).unwrap_err();

        assert!(matches!(error, ColorTableError::CannotRead { .. }));
        assert!(error.to_string().contains("33 MB"), "{error}");
    }

    #[test]
    fn a_folder_is_not_a_lut() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = folder.path().join("looks.cube");
        fs::create_dir(&file_path).unwrap();

        let error = load_color_lookup_table(&file_path).unwrap_err();

        assert!(error.to_string().contains("not a file"), "{error}");
    }

    #[test]
    fn the_library_reads_a_file_once_and_again_when_it_changes() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = write_file(&folder, "look.cube", INVERTING_CUBE);
        let mut library = ColorTableLibrary::default();

        let first = library.table_at(&file_path).unwrap();
        let second = library.table_at(&file_path).unwrap();
        assert!(Arc::ptr_eq(&first, &second), "read twice");

        fs::write(&file_path, UNTITLED_INVERTING_CUBE).unwrap();
        let rewritten = library.table_at(&file_path).unwrap();
        assert_eq!(rewritten.title(), None);
    }

    #[test]
    fn the_library_keeps_failures_and_reads_a_file_that_appears() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = folder.path().join("later.cube");
        let mut library = ColorTableLibrary::default();

        assert!(library.table_at(&file_path).is_err());
        assert!(library.table_at(&file_path).is_err());
        assert_eq!(library.kept_file_count(), 1);

        fs::write(&file_path, INVERTING_CUBE).unwrap();
        assert!(library.table_at(&file_path).is_ok());
    }

    #[test]
    fn the_library_forgets_files_the_project_no_longer_uses() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = write_file(&folder, "look.cube", INVERTING_CUBE);
        let mut library = ColorTableLibrary::default();
        library.table_at(&file_path).unwrap();

        library.forget_files_unused_by(&Project::new(
            kadr_core::project::ProjectSettings::landscape_1080p(),
        ));

        assert_eq!(library.kept_file_count(), 0);
    }

    #[test]
    fn only_the_first_enabled_lut_with_some_strength_applies() {
        use kadr_core::effect::{ColorAdjustment, Effect};
        use kadr_core::ids::EffectId;
        let effect = |id, is_enabled, kind| Effect {
            id: EffectId::from_raw(id),
            is_enabled,
            kind,
        };
        let lut = |name: &str, strength_fraction| {
            EffectKind::ColorLut(ColorLut {
                file_path: PathBuf::from(name),
                strength_fraction,
            })
        };
        let effects = vec![
            effect(
                1,
                true,
                EffectKind::ColorAdjustment(ColorAdjustment::NEUTRAL),
            ),
            effect(2, false, lut("off.cube", 1.0)),
            effect(3, true, lut("first.cube", 0.5)),
            effect(4, true, lut("second.cube", 1.0)),
        ];
        assert_eq!(
            applied_color_lut_of(&effects).map(|lut| lut.file_path.clone()),
            Some(PathBuf::from("first.cube"))
        );

        let effects = vec![
            effect(1, true, lut("none.cube", 0.0)),
            effect(2, true, lut("second.cube", 1.0)),
        ];
        assert_eq!(applied_color_lut_of(&effects), None);
    }
}
