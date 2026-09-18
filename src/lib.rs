//! codescape's renderer-free core: scanning, layout, atlas rasterisation,
//! overlays and scene geometry. The binary adds the Makepad renderer on top.
//!
//! Keeping these apart means the parts that decide how the tool scales can be
//! tested and benchmarked on a machine with no GPU and no windowing libraries.

pub mod atlas;
pub mod camera;
pub mod capture;
pub mod diff;
pub mod layout;
pub mod mem;
pub mod overlay;
pub mod scan;
pub mod scene;
pub mod tour;
pub mod trace;
