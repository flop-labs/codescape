//! codescape: fly through a git repository's source in 3D.
//!
//! Usage: codescape [REPO] [--tour] [--loop] [--record DIR] [--record-fps N] [--atlas N]

mod app;
mod atlas;
mod camera;
mod layout;
mod scan;
mod scape;
mod tour;

fn main() {
    app::app_main();
}
