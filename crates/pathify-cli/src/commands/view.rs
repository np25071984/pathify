use std::io::Write;

use anyhow::{Result, bail};
use pathify_core::formats;

use crate::cli::ViewArgs;
use crate::io::read_input;

pub fn run(args: &ViewArgs, _out: &mut dyn Write) -> Result<()> {
    // Checked before the file is even read: failing immediately with a clear
    // message beats letting the terminal library fail confusingly halfway
    // through, or painting a screen into a file nobody will look at.
    if !pathify_tui::is_interactive() {
        bail!(
            "`view` draws an interactive map and needs a terminal, but stdout is \
             redirected.\nRun it without a pipe, or use `pathify info` for output \
             you can redirect."
        );
    }

    let (bytes, path) = read_input(&args.input)?;
    let format = formats::detect(args.from, path.as_deref(), &bytes)?;
    let trace = formats::read(format, &bytes)?;

    let title = match path.as_deref() {
        Some(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        None => "stdin".to_string(),
    };

    if !pathify_tui::run(&trace, &title)? {
        bail!("nothing to draw: {title} has no points");
    }
    Ok(())
}
