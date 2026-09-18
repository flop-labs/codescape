//! codescape: fly through a git repository's source in 3D.
//!
//! Usage: codescape [REPO] [--diff REV|RANGE] [--pr N] [--tour] [--loop]
//!                  [--record DIR] [--record-fps N] [--atlas N]

mod app;
mod scape;

// Re-exported at the crate root so the renderer can keep saying `crate::`.
pub use codescape::{atlas, camera, capture, diff, layout, overlay, scan, scene, tour, trace};

/// Why the X display is unusable, if it is. Makepad opens it before any of
/// our code runs and segfaults rather than reporting a failure, so the two
/// cases worth naming are checked here: no `DISPLAY`, and a `DISPLAY` naming a
/// local server that is not listening.
#[cfg(target_os = "linux")]
fn display_problem() -> Option<String> {
    let Some(display) = std::env::var_os("DISPLAY") else {
        return Some("DISPLAY is not set".into());
    };
    let display = display.to_string_lossy().into_owned();
    // Only the local `:N[.S]` form maps to a socket we can check; anything
    // else is a remote or indirect display, so leave it to the X client.
    let Some(rest) = display.strip_prefix(':') else {
        return None;
    };
    let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if number.is_empty() {
        return None;
    }
    let socket = format!("/tmp/.X11-unix/X{number}");
    if std::path::Path::new(&socket).exists() {
        return None;
    }
    Some(format!("DISPLAY is {display}, but {socket} does not exist"))
}

fn main() {
    #[cfg(target_os = "linux")]
    if let Some(problem) = display_problem() {
        eprintln!("codescape: {problem}, so there is no X server to draw on.");
        eprintln!("  Log into a graphical session, or under Wayland run it through XWayland.");
        eprintln!("  For headless work, the renderer-free benchmark needs no display:");
        eprintln!("    cargo run --release --no-default-features --bin bench -- .");
        std::process::exit(2);
    }
    app::app_main();
}
