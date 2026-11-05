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
        acknowledge_input: false,
        output_limit: 4096,
        // Cold PowerShell startup on Windows CI can exceed ten seconds.
        timeout: Duration::from_secs(30),
    }
}

#[tokio::test]
async fn preserves_binary_stdio_and_reports_process_failure() {
    let mut invocation = command(
        "$inputStream = [Console]::OpenStandardInput(); $outputStream = [Console]::OpenStandardOutput(); $inputStream.CopyTo($outputStream)",
    );
    let bytes = [0, 255, 13, 10, 0, 128]
        .into_iter()
        .cycle()
        .take(128 * 1024)
        .collect::<Vec<_>>();
    invocation.output_limit = bytes.len();
    invocation.input = vec![WriteFrame {
        bytes: bytes.clone(),
        wait: Duration::ZERO,
    }];
    assert_eq!(
        Process
            .run(invocation, &CancellationToken::new())
            .await
            .unwrap(),
        bytes
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
    let mut invocation = command("[Console]::Write('x' * 4097); Start-Sleep -Seconds 120");
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
        .clone_from(&"[Console]::Error.Write('x' * 65537); Start-Sleep -Seconds 120".into());
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
        "[IO.File]::WriteAllText($args[0] + '.tmp', [string]$PID)\n[IO.File]::Move($args[0] + '.tmp', $args[0])\nStart-Sleep -Seconds 120\n",
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
        acknowledge_input: false,
        output_limit: 4096,
        timeout: Duration::from_secs(60),
    };
    let token = CancellationToken::new();
    let child_token = token.clone();
    let task = tokio::spawn(async move { Process.run(invocation, &child_token).await });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "Fake process did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    token.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("Cancellation did not reap the started process promptly")
            .unwrap(),
        Err(Error::Cancelled)
    ));
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
        acknowledge_input: false,
        output_limit: 4096,
        timeout: Duration::from_secs(30),
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

#[tokio::test]
async fn acknowledged_frames_start_their_hold_after_the_child_applies_them() {
    let mut invocation = command(concat!(
        "$reader = [IO.StreamReader]::new([Console]::OpenStandardInput()); ",
        "$writer = [Console]::OpenStandardOutput(); Start-Sleep -Milliseconds 350; ",
        "$clock = [Diagnostics.Stopwatch]::StartNew(); $times = @(); ",
        "for ($i = 0; $i -lt 3; $i++) { ",
        "if ($reader.ReadLine() -ne [string]$i) { exit 2 }; ",
        "$times += $clock.ElapsedMilliseconds; ",
        "Start-Sleep -Milliseconds 180; $writer.WriteByte(65); $writer.Flush(); }; ",
        "[Console]::Write(($times -join ','))"
    ));
    invocation.acknowledge_input = true;
    invocation.input = (0..3)
        .map(|i| WriteFrame {
            bytes: format!("{i}\n").into_bytes(),
            wait: Duration::from_millis(120),
        })
        .collect();
    let output = Process
        .run(invocation, &CancellationToken::new())
        .await
        .unwrap();
    let times = std::str::from_utf8(&output)
        .unwrap()
        .split(',')
        .map(|v| v.parse::<u64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(times.len(), 3);
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= 280,
            "child delays and holds must not be compressed: {times:?}"
        );
    }
}

#[tokio::test]
async fn acknowledged_streams_reject_missing_wrong_and_excess_output() {
    for (script, limit, output_limit) in [
        (
            "$null = [Console]::In.ReadLine(); [Console]::Write('X'); Start-Sleep -Seconds 60",
            4096,
            false,
        ),
        ("$null = [Console]::In.ReadLine(); exit 0", 4096, false),
        (
            "$null = [Console]::In.ReadLine(); [Console]::Write('AAA'); Start-Sleep -Seconds 60",
            1,
            true,
        ),
        (
            "$null = [Console]::In.ReadLine(); [Console]::Error.Write('x' * 65537); Start-Sleep -Seconds 60",
            4096,
            true,
        ),
    ] {
        let mut invocation = command(script);
        invocation.acknowledge_input = true;
        invocation.output_limit = limit;
        invocation.input = vec![WriteFrame {
            bytes: b"frame\n".to_vec(),
            wait: Duration::ZERO,
        }];
        let result = Process.run(invocation, &CancellationToken::new()).await;
        assert!(
            if output_limit {
                matches!(result, Err(Error::OutputLimit))
            } else {
                matches!(result, Err(Error::Device(_)))
            },
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn cancelling_an_acknowledged_frame_never_queues_the_next_one() {
    for acknowledged in [false, true] {
        let folder = tempfile::tempdir().unwrap();
        let script = folder.path().join("ack.ps1");
        let prefix = folder.path().join("child");
        std::fs::write(
            &script,
            concat!(
                "param($path, $ack)\n",
                "$reader = [IO.StreamReader]::new([Console]::OpenStandardInput())\n",
                "$writer = [Console]::OpenStandardOutput()\n",
                "$null = $reader.ReadLine()\n",
                "[IO.File]::WriteAllText($path + '.pid', [string]$PID)\n",
                "if ($ack -eq 'yes') { $writer.WriteByte(65); $writer.Flush() }\n",
                "[IO.File]::WriteAllText($path + '.ready', 'ready')\n",
                "$next = $reader.ReadLine()\n",
                "if ($null -ne $next) { [IO.File]::WriteAllText($path + '.extra', $next) }\n",
                "Start-Sleep -Seconds 120\n"
            ),
        )
        .unwrap();
        let mut invocation = command("");
        invocation.args = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.into_os_string(),
            prefix.clone().into_os_string(),
            if acknowledged { "yes" } else { "no" }.into(),
        ];
        invocation.acknowledge_input = true;
        invocation.input = vec![
            WriteFrame {
                bytes: b"first\n".to_vec(),
                wait: if acknowledged {
                    Duration::from_secs(5)
                } else {
                    Duration::ZERO
                },
            },
            WriteFrame {
                bytes: b"second\n".to_vec(),
                wait: Duration::ZERO,
            },
        ];
        let token = CancellationToken::new();
        let child_token = token.clone();
        let task = tokio::spawn(async move { Process.run(invocation, &child_token).await });
        let deadline = Instant::now() + Duration::from_secs(30);
        while !prefix.with_extension("ready").exists() {
            assert!(Instant::now() < deadline, "Fake ACK child did not start");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap(),
            Err(Error::Cancelled)
        ));
        assert!(
            !prefix.with_extension("extra").exists(),
            "the next input frame was queued"
        );
        let pid: u32 = std::fs::read_to_string(prefix.with_extension("pid"))
            .unwrap()
            .parse()
            .unwrap();
        Process
            .run(
                command(&format!(
                    "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }}"
                )),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
    }
}
