use anyhow::Result;
use crate::cli::WatchArgs;
use crate::i18n::text as t;
use crate::paths::GqyPaths;

/// 管家监控：采样系统进程，检测到异常时给运行中的 WebUI 会话入队主动消息。
pub fn run_watch(paths: &GqyPaths, args: WatchArgs) -> Result<()> {
    let interval = crate::alarm::parse_alarm_seconds(&args.every)?;
    loop {
        let sample = match crate::watch::sample_system() {
            Ok(sample) => sample,
            Err(err) => {
                eprintln!("watch: {err}");
                return Ok(());
            }
        };
        if crate::watch::should_alert(&sample) {
            let message = crate::watch::alert_message(&sample);
            let delivered = crate::watch::enqueue_alert(paths, &message)?;
            if delivered {
                println!(
                    "{}",
                    t(
                        "watch: anomaly detected, alert queued to the running session",
                        "监控：检测到异常，已给运行中的会话入队主动提醒"
                    )
                );
            } else {
                println!(
                    "{}",
                    t(
                        "watch: anomaly detected (WebUI not running, alert skipped)",
                        "监控：检测到异常（WebUI 未运行，跳过提醒）"
                    )
                );
            }
        }
        if args.once {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}
