#![cfg(windows)]

use coc_bot_core::{
    Error,
    device::process::{Invocation, Process, Transport, WriteFrame},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn powershell() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}

fn command(script: &str) -> Invocation {
    Invocation {
        program: powershell(),
        args: ["-NoProfile", "-NonInteractive", "-Command", script]
            .map(Into::into)
            .to_vec(),
        input: vec![],
        output_limit: 4096,
        timeout: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn preserves_binary_stdio_and_reports_process_failure() {
    let mut invocation = command(
        "$inputStream = [Console]::OpenStandardInput(); $outputStream = [Console]::OpenStandardOutput(); $inputStream.CopyTo($outputStream)",
    );
    invocation.input = vec![WriteFrame {
        bytes: vec![0, 255, 13, 10, 0, 128],
        wait: Duration::ZERO,
    }];
    assert_eq!(
        Process
            .run(invocation, &CancellationToken::new())
            .await
            .unwrap(),
        vec![0, 255, 13, 10, 0, 128]
    );
    assert!(matches!(
        Process
            .run(command("exit 7"), &CancellationToken::new())
            .await,
        Err(Error::Device(_))
    ));
}

#[tokio::test]
async fn bounds_both_output_streams_and_time() {
    let mut invocation = command("[Console]::Write('x' * 4097); Start-Sleep -Seconds 30");
    assert!(matches!(
        Process
            .run(invocation.clone(), &CancellationToken::new())
            .await,
        Err(Error::OutputLimit)
    ));
    invocation
        .args
        .last_mut()
        .unwrap()
        .clone_from(&"[Console]::Error.Write('x' * 65537); Start-Sleep -Seconds 30".into());
    assert!(matches!(
        Process.run(invocation, &CancellationToken::new()).await,
        Err(Error::OutputLimit)
    ));
    let mut invocation = command("Start-Sleep -Seconds 30");
    invocation.timeout = Duration::from_millis(100);
    assert!(matches!(
        Process.run(invocation, &CancellationToken::new()).await,
        Err(Error::Timeout)
    ));
}

#[tokio::test]
async fn cancellation_reaps_the_owned_process() {
    let folder = tempfile::tempdir().unwrap();
    let script = folder.path().join("sleep.ps1");
    let pid_file = folder.path().join("pid.txt");
    std::fs::write(
        &script,
        "[IO.File]::WriteAllText($args[0], [string]$PID)\nStart-Sleep -Seconds 30\n",
    )
    .unwrap();
    let invocation = Invocation {
        program: powershell(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.into_os_string(),
            pid_file.clone().into_os_string(),
        ],
        input: vec![],
        output_limit: 4096,
        timeout: Duration::from_secs(10),
    };
    let token = CancellationToken::new();
    let child_token = token.clone();
    let task = tokio::spawn(async move { Process.run(invocation, &child_token).await });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "Fake process did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    token.cancel();
    assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
    let check_script = folder.path().join("check.ps1");
    std::fs::write(
        &check_script,
        "if (Get-Process -Id ([int]$args[0]) -ErrorAction SilentlyContinue) { exit 1 }\n",
    )
    .unwrap();
    let check = Invocation {
        program: powershell(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            check_script.into_os_string(),
            pid.to_string().into(),
        ],
        input: vec![],
        output_limit: 4096,
        timeout: Duration::from_secs(10),
    };
    Process.run(check, &CancellationToken::new()).await.unwrap();
}

#[tokio::test]
async fn cancelled_or_unbounded_work_never_spawns() {
    let token = CancellationToken::new();
    token.cancel();
    let mut invocation = command("exit 0");
    invocation.program = PathBuf::from("does-not-exist.exe");
    assert!(matches!(
        Process.run(invocation.clone(), &token).await,
        Err(Error::Cancelled)
    ));
    invocation.input = vec![
        WriteFrame {
            bytes: vec![],
            wait: Duration::MAX
        };
        2
    ];
    assert!(matches!(
        Process.run(invocation, &CancellationToken::new()).await,
        Err(Error::Device(_))
    ));
}
