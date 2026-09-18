//! Saving the window as an image, for `--shot` and `--record`.
//!
//! The renderer has no readback of its own, so a frame is saved by asking the
//! platform's screenshot tool for the window. X11 names a window by its title;
//! macOS needs the window number, which only the renderer can look up.

use std::path::Path;
use std::process::Command;

/// A platform tool that saves one frame of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// X11: `xwd` finds the window by title.
    Xwd,
    /// macOS: `screencapture` takes the window's number (its CGWindowID).
    ScreenCapture(u32),
}

impl Grab {
    /// The format the tool writes, as a file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Grab::Xwd => "xwd",
            Grab::ScreenCapture(_) => "png",
        }
    }

    /// The command that saves the window titled `title` to `out`.
    pub fn command(self, title: &str, out: &Path) -> Command {
        match self {
            Grab::Xwd => {
                let mut c = Command::new("xwd");
                c.args(["-name", title, "-silent", "-out"]).arg(out);
                c
            }
            Grab::ScreenCapture(window) => {
                // -x: no shutter sound; -o: no drop shadow, so the image is
                // the window's own pixels, as xwd gives.
                let mut c = Command::new("screencapture");
                c.args(["-x", "-o", "-l", &window.to_string()]).arg(out);
                c
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(c: &Command) -> (String, Vec<String>) {
        (
            c.get_program().to_string_lossy().into_owned(),
            c.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
        )
    }

    #[test]
    fn xwd_names_the_window_by_title() {
        let c = Grab::Xwd.command("FLOP codescape", Path::new("/tmp/f00001.xwd"));
        assert_eq!(
            argv(&c),
            (
                "xwd".into(),
                vec![
                    "-name".into(),
                    "FLOP codescape".into(),
                    "-silent".into(),
                    "-out".into(),
                    "/tmp/f00001.xwd".into()
                ]
            )
        );
    }

    #[test]
    fn screencapture_names_the_window_by_number() {
        let c = Grab::ScreenCapture(7739).command("FLOP codescape", Path::new("/tmp/shot.png"));
        assert_eq!(
            argv(&c),
            (
                "screencapture".into(),
                vec![
                    "-x".into(),
                    "-o".into(),
                    "-l".into(),
                    "7739".into(),
                    "/tmp/shot.png".into()
                ]
            )
        );
    }

    #[test]
    fn frames_are_named_for_the_format_written() {
        assert_eq!(Grab::Xwd.extension(), "xwd");
        assert_eq!(Grab::ScreenCapture(1).extension(), "png");
    }
}
