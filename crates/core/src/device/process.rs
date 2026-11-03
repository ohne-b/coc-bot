use std::{ffi::OsString, future::Future, path::PathBuf, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct WriteFrame {
    pub bytes: Vec<u8>,
    pub wait: Duration,
}

#[derive(Clone, Debug)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub input: Vec<WriteFrame>,
    pub output_limit: usize,
    pub timeout: Duration,
}

/// One subprocess boundary shared by ADB and LDPlayer, replaceable in offline tests.
pub trait Transport: Send + Sync {
    fn run(
        &self,
        command: Invocation,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

#[derive(Default)]
pub struct Process;

impl Transport for Process {
    async fn run(&self, invocation: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if invocation.output_limit > 256 * 1024 * 1024 + 16
            || invocation.timeout.is_zero()
            || invocation.timeout > Duration::from_secs(60)
            || invocation.input.len() > 128
            || invocation.input.iter().fold(0_usize, |total, frame| {
                total.saturating_add(frame.bytes.len())
            }) > 256 * 1024
            || invocation
                .input
                .iter()
                .fold(Duration::ZERO, |total, frame| {
                    total.saturating_add(frame.wait)
                })
                > Duration::from_secs(12)
        {
            return Err(Error::Device("Invalid process limits".into()));
        }
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let work = async {
            let write = async {
                for frame in invocation.input {
                    stdin.write_all(&frame.bytes).await?;
                    stdin.flush().await?;
                    tokio::time::sleep(frame.wait).await;
                }
                stdin.shutdown().await?;
                drop(stdin);
                Ok::<_, Error>(())
            };
            let (_, output, _, status) = tokio::try_join!(
                write,
                read_bounded(stdout, invocation.output_limit),
                read_bounded(stderr, 64 * 1024),
                async { Ok::<_, Error>(child.wait().await?) },
            )?;
            if !status.success() {
                return Err(Error::Device(format!(
                    "Device command exited with {status}"
                )));
            }
            Ok(output)
        };
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(Error::Cancelled),
            _ = tokio::time::sleep(invocation.timeout) => Err(Error::Timeout),
            result = work => result,
        };
        if result.is_err() {
            // Kill and reap the owned client; never kill the shared ADB server.
            child.kill().await?;
        }
        result
    }
}

async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .await?;
    if output.len() > limit {
        return Err(Error::OutputLimit);
    }
    Ok(output)
}
