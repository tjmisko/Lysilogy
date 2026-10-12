//! Finite subprocess output capture. Only native text extraction may drain extra diagnostics.

use std::{process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command};

use crate::{Error, Result};

use super::COMMAND_TIMEOUT;

const STDERR_PREFIX_BYTES: usize = 64 * 1024;
const NATIVE_STDERR_DRAIN_BYTES: usize = 1024 * 1024;
const SCRATCH_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy)]
enum StderrPolicy {
    Strict,
    NativeDiagnostics,
}

impl StderrPolicy {
    const fn drain_limit(self) -> usize {
        match self {
            Self::Strict => STDERR_PREFIX_BYTES,
            Self::NativeDiagnostics => NATIVE_STDERR_DRAIN_BYTES,
        }
    }
}

#[derive(Debug)]
struct Diagnostics {
    prefix: Vec<u8>,
    bytes_observed: usize,
    drain_limit_bytes: usize,
    complete: bool,
}

impl Diagnostics {
    const fn truncated(&self) -> bool {
        self.bytes_observed > self.prefix.len()
    }

    fn summary(&self) -> String {
        format!(
            "stderr retained_bytes={} bytes_observed={} truncated={} drain_limit_bytes={} complete={}",
            self.prefix.len(),
            self.bytes_observed,
            self.truncated(),
            self.drain_limit_bytes,
            self.complete,
        )
    }

    fn report(&self, program: &str) {
        if self.bytes_observed != 0 {
            // ASCII escaping preserves every retained byte, including invalid UTF-8/control
            // bytes. It cannot turn deposited diagnostics into terminal control sequences.
            tracing::warn!(
                program,
                stderr_prefix = %self.prefix.escape_ascii(),
                retained_bytes = self.prefix.len(),
                bytes_observed = self.bytes_observed,
                truncated = self.truncated(),
                drain_limit_bytes = self.drain_limit_bytes,
                complete = self.complete,
                "native extractor diagnostics"
            );
        }
    }

    fn failure_text(&self) -> String {
        let prefix = String::from_utf8_lossy(&self.prefix).trim().to_owned();
        if self.truncated() {
            format!("{prefix}\n[{}]", self.summary())
        } else {
            prefix
        }
    }
}

#[derive(Debug)]
struct CommandOutput {
    stdout: Vec<u8>,
    diagnostics: Diagnostics,
}

/// Existing OCR/graphics callers retain their strict 64 KiB stderr limit.
pub(super) async fn bounded_command(
    command: &mut Command,
    program: &str,
    limit: usize,
) -> Result<Vec<u8>> {
    Ok(run(
        command,
        program,
        limit,
        StderrPolicy::Strict,
        COMMAND_TIMEOUT,
    )
    .await?
    .stdout)
}

/// The call site explicitly grants native pdftotext a finite diagnostic drainage budget.
pub(super) async fn native_command(command: &mut Command, limit: usize) -> Result<Vec<u8>> {
    let output = run(
        command,
        "pdftotext",
        limit,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await?;
    output.diagnostics.report("pdftotext");
    Ok(output.stdout)
}

async fn read_diagnostics(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    program: &str,
    policy: StderrPolicy,
) -> Result<Diagnostics> {
    let limit = policy.drain_limit();
    let mut diagnostics = Diagnostics {
        prefix: Vec::with_capacity(STDERR_PREFIX_BYTES),
        bytes_observed: 0,
        drain_limit_bytes: limit,
        complete: false,
    };
    let mut scratch = [0_u8; SCRATCH_BYTES];
    loop {
        // Read exactly at most limit+1 bytes. An overflowing count is a lower bound,
        // never a claim about bytes that the child emitted after this rejection.
        let available = SCRATCH_BYTES.min(limit + 1 - diagnostics.bytes_observed);
        let count = stream
            .read(&mut scratch[..available])
            .await
            .map_err(|error| Error::io(program, error))?;
        if count == 0 {
            diagnostics.complete = true;
            return Ok(diagnostics);
        }
        let retain = count.min(STDERR_PREFIX_BYTES - diagnostics.prefix.len());
        diagnostics.prefix.extend_from_slice(&scratch[..retain]);
        diagnostics.bytes_observed += count;
        if diagnostics.bytes_observed > limit {
            let detail = match policy {
                StderrPolicy::Strict => String::new(),
                StderrPolicy::NativeDiagnostics => {
                    diagnostics.report(program);
                    format!(" ({})", diagnostics.summary())
                }
            };
            return Err(Error::Task(format!(
                "{program} error output exceeded its bounded limit{detail}"
            )));
        }
    }
}

/// Child ownership encloses drains and wait, so errors, timeout and cancellation kill it.
async fn run(
    command: &mut Command,
    program: &str,
    limit: usize,
    policy: StderrPolicy,
    timeout: Duration,
) -> Result<CommandOutput> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::ProgramUnavailable(program.into())
            } else {
                Error::io(program, error)
            }
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Task("Command has no output pipe".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Task("Command has no error pipe".into()))?;
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io(program, error))?;
        if bytes.len() > limit {
            return Err(Error::Task(format!(
                "{program} output exceeded its bounded limit"
            )));
        }
        Ok(bytes)
    };
    tokio::time::timeout(timeout, async {
        let (stdout, diagnostics) =
            tokio::try_join!(read, read_diagnostics(stderr, program, policy))?;
        let status = child
            .wait()
            .await
            .map_err(|error| Error::io(program, error))?;
        if !status.success() {
            if matches!(policy, StderrPolicy::NativeDiagnostics) {
                diagnostics.report(program);
            }
            return Err(Error::CommandFailed {
                program: program.into(),
                status: status.to_string(),
                stderr: diagnostics.failure_text(),
            });
        }
        Ok(CommandOutput {
            stdout,
            diagnostics,
        })
    })
    .await
    .map_err(|_| {
        Error::Task(format!(
            "{program} timed out after {} seconds",
            timeout.as_secs()
        ))
    })?
}

#[cfg(test)]
mod tests;
