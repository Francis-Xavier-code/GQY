use anyhow::Result;
use crate::cli::SttArgs;
use crate::paths::GqyPaths;

pub fn run_stt(paths: &GqyPaths, args: SttArgs) -> Result<()> {
    let text = crate::speech::transcribe(paths, &args.audio, Some(&args.locale))?;
    println!("{text}");
    Ok(())
}
