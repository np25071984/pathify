use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result, anyhow};
use pathify_core::formats;

use crate::cli::ConvertArgs;
use crate::io::read_input;

pub fn run(args: &ConvertArgs, out: &mut dyn Write) -> Result<()> {
    let target = args.target_format().map_err(|message| anyhow!(message))?;

    let (bytes, path) = read_input(args.input.as_deref())?;
    let source = formats::detect(args.from, path.as_deref(), &bytes)?;
    let trace = formats::read(source, &bytes)?;

    match &args.output {
        Some(destination) => {
            let file = File::create(destination)
                .with_context(|| format!("failed to create {}", destination.display()))?;
            let mut writer = BufWriter::new(file);
            formats::write(target, &trace, &mut writer)?;
            writer
                .flush()
                .with_context(|| format!("failed to write {}", destination.display()))?;
        }
        None => formats::write(target, &trace, out)?,
    }
    Ok(())
}
