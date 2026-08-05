use anyhow::Result;
use crate::cli::{BackupArgs, BackupCommand};
use crate::i18n::text as t;
use crate::paths::GqyPaths;

pub fn run_backup(paths: &GqyPaths, args: BackupArgs) -> Result<()> {
    match args.command {
        BackupCommand::Init(args) => {
            let remote = args
                .remote
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let local_mode = remote.is_none();
            crate::backup::init(
                paths,
                crate::backup::BackupInitOptions {
                    remote,
                    branch: args.branch,
                    git_name: args.name,
                    git_email: args.email,
                    auto_push: !args.no_auto_push,
                    ssh_key: args.ssh_key,
                },
            )?;
            if local_mode {
                println!(
                    "{}",
                    t(
                        "initialized local Git backup; attach a remote later with `gqy backup remote <url>`",
                        "已初始化本地 Git 备份；之后可用 `gqy backup remote <url>` 绑定远程"
                    )
                );
            } else {
                println!(
                    "{}",
                    t(
                        "initialized isolated Git backup; run `gqy backup now` after remote authentication is ready",
                        "已初始化独立 Git 备份；远程认证就绪后运行 `gqy backup now`"
                    )
                );
            }
        }
        BackupCommand::Now(args) => {
            let outcome = crate::backup::backup_now(paths, !args.no_push)?;
            print_backup_outcome(&outcome);
        }
        BackupCommand::Status => println!("{}", crate::backup::status(paths)?),
        BackupCommand::Restore(args) => {
            crate::backup::restore(
                paths,
                crate::backup::RestoreOptions {
                    remote: args.remote,
                    branch: args.branch,
                    git_name: args.name,
                    git_email: args.email,
                    ssh_key: args.ssh_key,
                    auto_push: !args.no_auto_push,
                    force: args.force,
                },
            )?;
            println!(
                "{}",
                t(
                    "restored GQY state from the backup remote; re-enter API keys if the restored config redacted them",
                    "已从备份远程恢复 GQY 状态；若恢复的配置脱敏了密钥，请重新填写"
                )
            );
        }
        BackupCommand::Remote(args) => {
            let auto_push = match (args.auto_push, args.no_auto_push) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                _ => None,
            };
            crate::backup::set_remote(paths, args.url, args.ssh_key, auto_push)?;
            println!(
                "{}",
                t(
                    "backup remote updated; next `gqy backup now` will push to it (supports `owner/repo` via gh CLI)",
                    "备份远程已更新；下一次 `gqy backup now` 将推送到该远程（支持用 gh CLI 传 `owner/repo`）"
                )
            );
        }
    }
    Ok(())
}

fn print_backup_outcome(outcome: &crate::backup::BackupOutcome) {
    let commit = outcome.commit.as_deref().unwrap_or("-");
    println!(
        "{}: {} · {}: {} · {}: {}",
        t("commit", "提交"),
        commit,
        t("new snapshot", "新快照"),
        outcome.committed,
        t("pushed", "已推送"),
        outcome.pushed
    );
}
