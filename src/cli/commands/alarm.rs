use anyhow::Result;
use crate::cli::{AlarmArgs, AlarmCommand, AlarmWorkerArgs};
use crate::i18n::text as t;
use crate::paths::GqyPaths;
use std::io::Write;
use std::time::Duration;
use std::path::PathBuf;

pub fn run_alarm_cmd(paths: &GqyPaths, args: AlarmArgs) -> Result<()> {
    match args.command {
        AlarmCommand::List => {
            let records = crate::alarm::cleanup_dead(paths)?;
            if records.is_empty() {
                println!("{}", t("No alarms.", "暂无闹钟。"));
                return Ok(());
            }
            for record in records {
                let repeat = if record.repeat_seconds > 0 {
                    format!(" · 每 {}s 一次", record.repeat_seconds)
                } else {
                    String::new()
                };
                println!(
                    "{}  {}  {}  {}{repeat}",
                    record.id,
                    crate::alarm::format_due_at(record.due_at),
                    record.label,
                    match record.status {
                        crate::alarm::AlarmStatus::Scheduled => "scheduled",
                        crate::alarm::AlarmStatus::Ringing => "ringing",
                    }
                );
            }
            Ok(())
        }
        AlarmCommand::Cancel { id } => {
            let cancelled = crate::alarm::cancel(paths, &id)?;
            if cancelled {
                println!("{}", t("Alarm cancelled.", "闹钟已取消。"));
            } else {
                println!("{}", t("Alarm not found.", "未找到该闹钟。"));
            }
            Ok(())
        }
        AlarmCommand::Stop { all } => {
            if !all {
                println!(
                    "{}",
                    t("use `gqy alarm stop --all` to stop all alarm workers", "使用 `gqy alarm stop --all` 停止全部闹钟")
                );
                return Ok(());
            }
            let stopped = crate::alarm::stop_all(paths)?;
            if crate::i18n::is_zh() {
                println!("已停止 {stopped} 个闹钟进程。");
            } else {
                println!("Stopped {stopped} alarm worker(s).");
            }
            Ok(())
        }
    }
}

pub fn run_alarm_worker(args: AlarmWorkerArgs) -> Result<()> {
    let paths = alarm_worker_paths(args.state_dir, args.cache_dir);
    let _worker_lock = crate::alarm::WorkerLock::acquire(&paths, &args.id)?;
    let _ = crate::alarm::write_pid_file(&paths, &args.id, std::process::id());
    let source = args
        .audio_file
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "builtin".to_string());
    let mut interval = crate::alarm::parse_alarm_seconds(&args.time)?;
    let mut rings = 0u64;
    loop {
        if !still_registered(&paths, &args.id) {
            crate::alarm::remove_pid_file(&paths, &args.id);
            let _ = append_alarm_log(
                &paths,
                &format!("{}: cancelled (no longer registered), exiting\n", args.id),
            );
            return Ok(());
        }
        if args.repeat > 0 && args.parent_pid != 0 && !parent_alive(args.parent_pid) {
            crate::alarm::remove_pid_file(&paths, &args.id);
            let _ = crate::alarm::remove(&paths, &args.id);
            let _ = append_alarm_log(
                &paths,
                &format!("{}: parent process exited, stopping repeating alarm\n", args.id),
            );
            return Ok(());
        }
        let _ = append_alarm_log(
            &paths,
            &format!("{}: scheduled in {interval}s; source={source}\n", args.id),
        );
        std::thread::sleep(Duration::from_secs(interval));
        if !still_registered(&paths, &args.id) {
            crate::alarm::remove_pid_file(&paths, &args.id);
            let _ = append_alarm_log(
                &paths,
                &format!("{}: cancelled while waiting, exiting\n", args.id),
            );
            return Ok(());
        }
        let _ = crate::alarm::update_status(&paths, &args.id, crate::alarm::AlarmStatus::Ringing);
        let _ = append_alarm_log(&paths, &format!("{}: playback starting\n", args.id));
        let result = play_alarm_once(args.audio_file.as_deref()).or_else(|err| {
            append_alarm_log(
                &paths,
                &format!("{}: audio playback failed: {err}\n", args.id),
            )?;
            terminal_bell_fallback();
            Ok(())
        });
        if result.is_ok() {
            let _ = append_alarm_log(&paths, &format!("{}: playback finished\n", args.id));
        }
        rings += 1;
        if args.repeat == 0 {
            crate::alarm::remove_pid_file(&paths, &args.id);
            let _ = crate::alarm::remove(&paths, &args.id);
            return result;
        }
        if args.max_rings > 0 && rings >= args.max_rings {
            crate::alarm::remove_pid_file(&paths, &args.id);
            let _ = crate::alarm::remove(&paths, &args.id);
            let _ = append_alarm_log(
                &paths,
                &format!("{}: reached max rings ({rings}), stopping\n", args.id),
            );
            return Ok(());
        }
        interval = args.repeat;
        let _ = crate::alarm::update_status(&paths, &args.id, crate::alarm::AlarmStatus::Scheduled);
    }
}

fn parent_alive(pid: u32) -> bool {
    crate::alarm::process_exists(pid)
}

fn still_registered(paths: &GqyPaths, id: &str) -> bool {
    crate::alarm::load(paths)
        .map(|records| records.iter().any(|record| record.id == id))
        .unwrap_or(false)
}

fn play_alarm_once(audio_file: Option<&std::path::Path>) -> Result<()> {
    const ALARM_WAV: &[u8] = include_bytes!("../../assets/alarm.wav");
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let audio = match audio_file {
        Some(path) => std::fs::read(path)?,
        None => ALARM_WAV.to_vec(),
    };
    let cursor = std::io::Cursor::new(audio);
    let sink = rodio::Sink::try_new(&handle)?;
    let source = rodio::Decoder::new(cursor)?;
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}

fn terminal_bell_fallback() {
    for _ in 0..5 {
        let _ = std::io::stderr().write_all(b"\x07");
        let _ = std::io::stderr().flush();
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn append_alarm_log(paths: &GqyPaths, line: &str) -> Result<()> {
    std::fs::create_dir_all(paths.logs_dir())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::alarm::alarm_log_file(paths))?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn alarm_worker_paths(state_dir: PathBuf, cache_dir: PathBuf) -> GqyPaths {
    let share_dir = crate::paths::resolve_share_base();
    GqyPaths {
        config_dir: PathBuf::new(),
        config_file: PathBuf::new(),
        skills_dir: PathBuf::new(),
        data_dir: PathBuf::new(),
        cache_dir,
        state_dir,
        pictures_dir: PathBuf::new(),
        fish_hook_file: PathBuf::new(),
        bash_hook_file: PathBuf::new(),
        zsh_hook_file: PathBuf::new(),
        scripts_dir: PathBuf::new(),
        system_scripts_dir: share_dir.join("scripts"),
        share_dir,
        kb_dir: PathBuf::new(),
    }
}
