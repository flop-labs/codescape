//! Per-line marks laid over the source text.
//!
//! A mark recolours one line of one file. The minimap raster bakes marks once
//! at load, and the glyph collector reads them again per frame, so an overlay
//! rides on the two draw calls that already exist and costs no extra state.
//! Both `diff` and `trace` produce one.

pub const M_NONE: u8 = 0;
/// Line is new in the head revision.
pub const M_ADDED: u8 = 1;
/// Line is new, and replaces one that was removed.
pub const M_CHANGED: u8 = 2;
/// Lines were removed at this point; the mark sits on the line that closed the gap.
pub const M_GONE: u8 = 3;
/// Quint: a variable written by the current step is declared or assigned here.
pub const M_STEP: u8 = 4;
/// Quint: written by an earlier step of the trace.
pub const M_PAST: u8 = 5;
pub const MARK_COUNT: usize = 6;

/// Colour per mark. `M_NONE` is never looked up; it keeps the syntax colour.
pub fn mark_palette() -> [[f32; 3]; MARK_COUNT] {
    [
        [0.0, 0.0, 0.0], // none — unused
        hex(0x32D74B),   // added — Electric Green
        hex(0xF2A33C),   // changed — amber
        hex(0xE5484D),   // gone — red
        hex(0x00E5FF),   // step — bright cyan
        hex(0x4E7BA6),   // past — muted blue
    ]
}

fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileState {
    Added,
    Modified,
    Deleted,
}

impl FileState {
    pub fn tag(&self) -> &'static str {
        match self {
            FileState::Added => "A",
            FileState::Modified => "M",
            FileState::Deleted => "D",
        }
    }
}

pub struct FileOverlay {
    pub state: FileState,
    pub added: usize,
    pub removed: usize,
    /// One mark per line of the file as it is on disk.
    pub marks: Vec<u8>,
}

impl FileOverlay {
    pub fn churn(&self) -> usize {
        self.added + self.removed
    }
}

/// Marks for the whole scene, indexed like `Scene::sources`.
pub struct Overlay {
    pub files: Vec<Option<FileOverlay>>,
    /// Paths that exist only in the base revision, so they have no tile.
    pub deleted: Vec<(String, usize)>,
    /// Shown in the HUD: what this overlay is.
    pub label: String,
}

impl Overlay {
    pub fn empty(n: usize) -> Overlay {
        Overlay {
            files: (0..n).map(|_| None).collect(),
            deleted: Vec::new(),
            label: String::new(),
        }
    }

    /// Mark on `line` of source `file`, or `M_NONE`.
    #[inline]
    pub fn mark(&self, file: usize, line: usize) -> u8 {
        match &self.files[file] {
            Some(f) => f.marks.get(line).copied().unwrap_or(M_NONE),
            None => M_NONE,
        }
    }

    pub fn touched(&self) -> usize {
        self.files.iter().filter(|f| f.is_some()).count()
    }

    pub fn totals(&self) -> (usize, usize) {
        self.files
            .iter()
            .flatten()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed))
    }
}
