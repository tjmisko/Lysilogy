use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

use super::*;

fn python(source: &str) -> Command {
    let mut command = Command::new("python3");
    command.args(["-I", "-S", "-c", source]);
    command
}

#[tokio::test]
async fn should_retain_exact_prefix_and_count_when_native_diagnostics_cross_boundaries() {
    for size in [
        0,
        STDERR_PREFIX_BYTES - 1,
        STDERR_PREFIX_BYTES,
        STDERR_PREFIX_BYTES + 1,
        NATIVE_STDERR_DRAIN_BYTES - 1,
        NATIVE_STDERR_DRAIN_BYTES,
    ] {
        let bytes: Vec<u8> = (0_u8..=255).cycle().take(size).collect();
        let result = read_diagnostics(bytes.as_slice(), "fixture", StderrPolicy::NativeDiagnostics)
            .await
            .unwrap();
        assert_eq!(result.prefix, bytes[..size.min(STDERR_PREFIX_BYTES)]);
        assert_eq!(result.prefix.capacity(), STDERR_PREFIX_BYTES);
        assert_eq!(result.bytes_observed, size);
        assert_eq!(result.drain_limit_bytes, NATIVE_STDERR_DRAIN_BYTES);
        assert_eq!(result.truncated(), size > STDERR_PREFIX_BYTES);
        assert!(result.complete);
    }
}

#[tokio::test]
async fn should_stop_at_limit_plus_one_when_native_drain_budget_is_exceeded() {
    let bytes = vec![b'x'; NATIVE_STDERR_DRAIN_BYTES + 100];
    let mut remaining = bytes.as_slice();
    let error = read_diagnostics(&mut remaining, "fixture", StderrPolicy::NativeDiagnostics)
        .await
        .unwrap_err();
    assert_eq!(remaining.len(), 99);
    let message = error.to_string();
    assert!(message.contains("bytes_observed=1048577"));
    assert!(message.contains("retained_bytes=65536"));
    assert!(message.contains("complete=false"));
    assert!(message.contains("truncated=true"));
}

#[tokio::test]
async fn should_preserve_strict_stderr_boundary_when_existing_callers_do_not_opt_in() {
    let bytes = vec![b'x'; STDERR_PREFIX_BYTES + 1];
    let exact = read_diagnostics(
        &bytes[..STDERR_PREFIX_BYTES],
        "fixture",
        StderrPolicy::Strict,
    )
    .await
    .unwrap();
    assert_eq!(exact.prefix, bytes[..STDERR_PREFIX_BYTES]);
    assert!(!exact.truncated());
    let error = read_diagnostics(bytes.as_slice(), "fixture", StderrPolicy::Strict)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "task failed: fixture error output exceeded its bounded limit"
    );
}

#[tokio::test]
async fn should_keep_default_policy_when_program_name_matches_native_extractor() {
    let mut command = python("import sys;sys.stderr.buffer.write(b'x'*70000)");
    let error = bounded_command(&mut command, "pdftotext", 1024)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "task failed: pdftotext error output exceeded its bounded limit"
    );
}

#[derive(Clone)]
struct LogBytes(Arc<Mutex<Vec<u8>>>);

impl Write for LogBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn should_report_lossless_escaped_prefix_and_counts_when_successful_diagnostics_are_truncated() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogBytes(bytes.clone());
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let diagnostics = Diagnostics {
        prefix: vec![b'a', b'\n', 0xff, 0x1b, b'\\'],
        bytes_observed: 10,
        drain_limit_bytes: NATIVE_STDERR_DRAIN_BYTES,
        complete: true,
    };
    tracing::subscriber::with_default(subscriber, || diagnostics.report("pdftotext"));
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(log.contains("a\\n\\xff\\x1b\\\\"), "{log}");
    for field in [
        "retained_bytes=5",
        "bytes_observed=10",
        "truncated=true",
        "drain_limit_bytes=1048576",
        "complete=true",
    ] {
        assert!(log.contains(field), "{log}");
    }
    assert!(!log.as_bytes().contains(&0x1b));
    assert_eq!(log.lines().count(), 1);
}

#[tokio::test]
async fn should_return_identical_stdout_when_successful_native_stderr_is_large() {
    let mut command = python(
        "import sys; sys.stderr.buffer.write(b'x'*169653); sys.stdout.buffer.write(b'native\\x00output')",
    );
    let output = run(
        &mut command,
        "fixture",
        1024,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(output.stdout, b"native\x00output");
    assert_eq!(output.diagnostics.prefix, vec![b'x'; STDERR_PREFIX_BYTES]);
    assert_eq!(output.diagnostics.bytes_observed, 169_653);
    assert!(output.diagnostics.complete && output.diagnostics.truncated());
}

#[tokio::test]
async fn should_preserve_nonzero_exit_when_stderr_is_truncated() {
    let mut command =
        python("import sys; sys.stderr.buffer.write(b'failure:'+b'x'*70000); sys.exit(7)");
    let error = run(
        &mut command,
        "fixture",
        1024,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await
    .unwrap_err();
    let Error::CommandFailed { status, stderr, .. } = error else {
        panic!("nonzero exit was not preserved: {error}");
    };
    assert!(status.contains('7'));
    assert!(stderr.starts_with("failure:"));
    assert!(stderr.contains("bytes_observed=70008"));
    assert!(stderr.contains("truncated=true"));
    assert!(stderr.contains("complete=true"));
    assert!(stderr.len() < STDERR_PREFIX_BYTES + 200);
}

#[tokio::test]
async fn should_drain_both_pipes_when_concurrent_output_exceeds_pipe_capacity() {
    let mut command = python(
        "import sys,threading\na=threading.Thread(target=lambda:sys.stdout.buffer.write(b'o'*262144))\nb=threading.Thread(target=lambda:sys.stderr.buffer.write(b'e'*262144))\na.start();b.start();a.join();b.join()",
    );
    let output = run(
        &mut command,
        "fixture",
        262_144,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(output.stdout, vec![b'o'; 262_144]);
    assert_eq!(output.diagnostics.prefix, vec![b'e'; STDERR_PREFIX_BYTES]);
    assert_eq!(output.diagnostics.bytes_observed, 262_144);
}

#[tokio::test]
async fn should_preserve_output_and_error_text_when_diagnostics_fit_existing_limits() {
    for policy in [StderrPolicy::Strict, StderrPolicy::NativeDiagnostics] {
        let mut command = python(
            "import sys;sys.stdout.buffer.write(b'ordinary\\n');sys.stderr.buffer.write(b' warning\\n')",
        );
        let output = run(&mut command, "fixture", 9, policy, COMMAND_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(output.stdout, b"ordinary\n");
        assert_eq!(output.diagnostics.failure_text(), "warning");
        assert!(!output.diagnostics.truncated());
        let mut command = python("import sys;sys.stderr.buffer.write(b' warning\\n');sys.exit(1)");
        assert!(
            matches!(run(&mut command, "fixture", 9, policy, COMMAND_TIMEOUT).await,
            Err(Error::CommandFailed { stderr, .. }) if stderr == "warning")
        );
    }
}

#[cfg(target_os = "linux")]
async fn fixture_pid(path: &std::path::Path) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = tokio::fs::read_to_string(path).await
                && !pid.is_empty()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[cfg(target_os = "linux")]
async fn assert_child_gone(pid: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::fs::try_exists(format!("/proc/{pid}")).await.unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned command child survived failure/cancellation");
}

#[cfg(target_os = "linux")]
fn sleeping_child(pid: &std::path::Path, output: &str) -> Command {
    let mut command = python(&format!(
        "import os,sys,time\nopen(sys.argv[1],'w').write(str(os.getpid()))\n{output}\ntime.sleep(60)"
    ));
    command.arg(pid);
    command
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn should_kill_child_when_stdout_exceeds_limit_despite_allowed_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pid");
    let mut command = sleeping_child(
        &path,
        "sys.stderr.buffer.write(b'e'*70000);sys.stderr.flush();sys.stdout.buffer.write(b'o'*65537);sys.stdout.flush()",
    );
    let error = run(
        &mut command,
        "fixture",
        65536,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("fixture output exceeded"));
    assert_child_gone(&fixture_pid(&path).await).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn should_kill_child_when_stderr_exceeds_native_drain_budget() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pid");
    let mut command = sleeping_child(
        &path,
        "sys.stderr.buffer.write(b'e'*1048577);sys.stderr.flush()",
    );
    let error = run(
        &mut command,
        "fixture",
        1024,
        StderrPolicy::NativeDiagnostics,
        COMMAND_TIMEOUT,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("bytes_observed=1048577"));
    assert_child_gone(&fixture_pid(&path).await).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn should_kill_child_when_command_deadline_expires() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pid");
    let mut command = sleeping_child(
        &path,
        "sys.stderr.buffer.write(b'e'*70000);sys.stderr.flush()",
    );
    let task = tokio::spawn(async move {
        run(
            &mut command,
            "fixture",
            1024,
            StderrPolicy::NativeDiagnostics,
            Duration::from_secs(1),
        )
        .await
    });
    let pid = fixture_pid(&path).await;
    let error = task.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert_child_gone(&pid).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn should_kill_child_when_command_future_is_cancelled() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pid");
    let mut command = sleeping_child(
        &path,
        "sys.stderr.buffer.write(b'e'*70000);sys.stderr.flush()",
    );
    let task = tokio::spawn(async move {
        run(
            &mut command,
            "fixture",
            1024,
            StderrPolicy::NativeDiagnostics,
            COMMAND_TIMEOUT,
        )
        .await
    });
    let pid = fixture_pid(&path).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_child_gone(&pid).await;
}
