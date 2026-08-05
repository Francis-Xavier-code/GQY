use anyhow::Result;
use crate::cli::TtsArgs;
use crate::i18n::text as t;

pub fn run_tts(args: TtsArgs) -> Result<()> {
    if args.list {
        for voice in crate::speech::list_voices()? {
            println!("{voice}");
        }
        return Ok(());
    }
    if args.clone {
        crate::speech::speak_clone(&args.text, None)?;
        return Ok(());
    }
    crate::speech::speak(&args.text, args.voice.as_deref(), args.output.as_deref())?;
    if let Some(output) = &args.output {
        println!("{}", t("saved audio", "已生成音频"));
        println!("{output}");
    }
    Ok(())
}
