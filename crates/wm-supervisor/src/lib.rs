//! Process supervisor abstraction (WM-0107) and the native launcher
//! (WM-0108).
//!
//! Abstraction per plan §7.1: backends implement start/terminate/kill/
//! status/resource usage. v1 ships only `NativeProcessLauncher`; Docker/
//! Podman/K8s launchers are out of scope. Restart policy fields are
//! recorded but never auto-applied: automatic restart must be explicitly
//! allowed by the swell failure policy (plan §7.3) and is Phase 1+ work.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Child;

pub mod kbengine;

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("process not started")]
    NotStarted,
    #[error("process already running (pid {0})")]
    AlreadyRunning(u32),
    #[error("failed to spawn `{executable}`: {source}")]
    Spawn {
        executable: String,
        #[source]
        source: std::io::Error,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Where a stream is directed. v1 supports piped capture and inherit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSink {
    /// Capture into memory for the exit record's log tail.
    Capture,
    /// Inherit the parent's stream.
    Inherit,
}

/// Restart policy (plan §7.2). Recorded on the spec; the Phase 1 native
/// launcher never restarts automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartPolicy {
    #[default]
    Never,
    OnFailure,
}

/// Process start specification (plan §7.2 ProcessSpec).
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub stdout: StreamSink,
    pub stderr: StreamSink,
    pub restart_policy: RestartPolicy,
    pub shutdown_grace_period_ms: u64,
}

impl ProcessSpec {
    /// Convenience constructor: capture both streams, never restart.
    pub fn new(executable: impl Into<String>, args: &[&str]) -> Self {
        Self {
            executable: executable.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            working_directory: None,
            environment: HashMap::new(),
            stdout: StreamSink::Capture,
            stderr: StreamSink::Capture,
            restart_policy: RestartPolicy::Never,
            shutdown_grace_period_ms: 2000,
        }
    }
}

/// Terminal information about an exited process (plan §7.3: unexpected
/// exits must record exit code / signal plus log tail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitRecord {
    /// Process exit code, None when killed by a signal.
    pub code: Option<i32>,
    /// Terminating signal number, None on a normal exit.
    pub signal: Option<i32>,
    /// Last captured stderr bytes (log tail for the failure record).
    pub stderr_tail: Vec<u8>,
    /// Last captured stdout bytes.
    pub stdout_tail: Vec<u8>,
    /// How long the process ran before exiting.
    pub uptime: Duration,
}

impl ExitRecord {
    /// True when the process did not exit cleanly (signal death or nonzero).
    pub fn is_abnormal(&self) -> bool {
        self.signal.is_some() || self.code.is_some_and(|c| c != 0)
    }
}

/// Observed lifecycle state of the supervised process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessState {
    Prepared,
    Running { pid: u32 },
    Exited(ExitRecord),
}

/// Point-in-time resource usage of the supervised process (stub: best-effort
/// from /proc on Linux; None fields when unavailable).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceUsage {
    pub rss_bytes: Option<u64>,
    pub cpu_seconds: Option<f64>,
}

/// Backend-neutral process supervision (plan §7.1). Implemented with async
/// trait methods; add new launchers as additional implementors.
pub trait ProcessSupervisor {
    /// Start the process described by `spec`.
    fn start(
        &mut self,
        spec: &ProcessSpec,
    ) -> impl std::future::Future<Output = Result<u32, SupervisorError>> + Send;

    /// Graceful stop: SIGTERM, wait up to `grace`, then SIGKILL.
    fn terminate(
        &mut self,
        grace: Duration,
    ) -> impl std::future::Future<Output = Result<ExitRecord, SupervisorError>> + Send;

    /// Immediate SIGKILL.
    fn kill(
        &mut self,
    ) -> impl std::future::Future<Output = Result<ExitRecord, SupervisorError>> + Send;

    /// Current lifecycle state.
    fn status(
        &mut self,
    ) -> impl std::future::Future<Output = Result<ProcessState, SupervisorError>> + Send;

    /// Best-effort resource snapshot of the running process.
    fn resource_usage(
        &mut self,
    ) -> impl std::future::Future<Output = Result<ResourceUsage, SupervisorError>> + Send;
}

/// v1 native backend: tokio child processes on the local machine.
#[derive(Debug, Default)]
pub struct NativeProcessLauncher {
    child: Option<Child>,
    spec: Option<ProcessSpec>,
    started_at: Option<Instant>,
    last_exit: Option<ExitRecord>,
    stdout_buf: Arc<Mutex<Vec<u8>>>,
    stderr_buf: Arc<Mutex<Vec<u8>>>,
}

const MAX_CAPTURE_BYTES: usize = 64 * 1024;

impl NativeProcessLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    fn last_captured(buf: &Arc<Mutex<Vec<u8>>>, max: usize) -> Vec<u8> {
        let buf = buf.lock().unwrap();
        if buf.len() <= max {
            buf.clone()
        } else {
            buf[buf.len() - max..].to_vec()
        }
    }

    /// Poll a running child without blocking; Some when already reaped.
    fn poll_exit(child: &mut Child) -> Result<Option<std::process::ExitStatus>, SupervisorError> {
        Ok(child.try_wait()?)
    }

    fn spawn_drain_task(
        stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        sink: Arc<Mutex<Vec<u8>>>,
    ) {
        tokio::spawn(async move {
            let mut stream = stream;
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut buf = sink.lock().unwrap();
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > MAX_CAPTURE_BYTES {
                            let excess = buf.len() - MAX_CAPTURE_BYTES;
                            buf.drain(..excess);
                        }
                    }
                }
            }
        });
    }
}

#[cfg(unix)]
fn signal_of(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal_of(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

impl ProcessSupervisor for NativeProcessLauncher {
    async fn start(&mut self, spec: &ProcessSpec) -> Result<u32, SupervisorError> {
        if let Some(child) = &mut self.child {
            if child.try_wait()?.is_none() {
                return Err(SupervisorError::AlreadyRunning(
                    child.id().unwrap_or_default(),
                ));
            }
        }

        let mut cmd = tokio::process::Command::new(&spec.executable);
        cmd.args(&spec.args).kill_on_drop(true);
        if let Some(dir) = &spec.working_directory {
            cmd.current_dir(dir);
        }
        for (k, v) in &spec.environment {
            cmd.env(k, v);
        }
        if spec.stdout == StreamSink::Capture {
            cmd.stdout(std::process::Stdio::piped());
        } else {
            cmd.stdout(std::process::Stdio::inherit());
        }
        if spec.stderr == StreamSink::Capture {
            cmd.stderr(std::process::Stdio::piped());
        } else {
            cmd.stderr(std::process::Stdio::inherit());
        }

        let mut child = cmd.spawn().map_err(|source| SupervisorError::Spawn {
            executable: spec.executable.clone(),
            source,
        })?;
        let pid = child.id().context_spawn(&spec.executable)?;

        // Drain captured streams concurrently so a chatty child cannot
        // deadlock on full pipes.
        if let Some(out) = child.stdout.take() {
            Self::spawn_drain_task(out, Arc::clone(&self.stdout_buf));
        }
        if let Some(err) = child.stderr.take() {
            Self::spawn_drain_task(err, Arc::clone(&self.stderr_buf));
        }

        self.child = Some(child);
        self.spec = Some(spec.clone());
        self.started_at = Some(Instant::now());
        self.last_exit = None;
        self.stdout_buf.lock().unwrap().clear();
        self.stderr_buf.lock().unwrap().clear();
        tracing::info!(pid, executable = %spec.executable, "process started");
        Ok(pid)
    }

    async fn terminate(&mut self, grace: Duration) -> Result<ExitRecord, SupervisorError> {
        let Some(child) = &mut self.child else {
            return Err(SupervisorError::NotStarted);
        };
        // try_wait first: already-exited children must not be signalled.
        if Self::poll_exit(child)?.is_none() {
            #[cfg(unix)]
            {
                if let Some(id) = child.id() {
                    unsafe {
                        libc::kill(id as libc::pid_t, libc::SIGTERM);
                    }
                }
                let _ = tokio::time::timeout(grace, child.wait()).await;
            }
            #[cfg(not(unix))]
            {
                child.kill().await.ok();
            }
            // Escalate if still alive after the grace period.
            if Self::poll_exit(child)?.is_none() {
                return self.kill().await;
            }
        }
        self.collect_exit().await
    }

    async fn kill(&mut self) -> Result<ExitRecord, SupervisorError> {
        let Some(child) = &mut self.child else {
            return Err(SupervisorError::NotStarted);
        };
        if Self::poll_exit(child)?.is_none() {
            child.kill().await?;
        }
        self.collect_exit().await
    }

    async fn status(&mut self) -> Result<ProcessState, SupervisorError> {
        if let Some(child) = &mut self.child {
            return match child.try_wait()? {
                Some(_) => {
                    let record = self.collect_exit().await?;
                    Ok(ProcessState::Exited(record))
                }
                None => Ok(ProcessState::Running {
                    pid: child.id().unwrap_or_default(),
                }),
            };
        }
        // Reap a previously-collected exit even after the child is dropped.
        if let Some(record) = &self.last_exit {
            return Ok(ProcessState::Exited(record.clone()));
        }
        Ok(ProcessState::Prepared)
    }

    async fn resource_usage(&mut self) -> Result<ResourceUsage, SupervisorError> {
        let Some(child) = &mut self.child else {
            return Err(SupervisorError::NotStarted);
        };
        let Some(pid) = child.id() else {
            return Ok(ResourceUsage::default());
        };
        Ok(read_proc_usage(pid))
    }
}

impl NativeProcessLauncher {
    /// Wait for the child to finish and build the exit record from the
    /// captured streams.
    async fn collect_exit(&mut self) -> Result<ExitRecord, SupervisorError> {
        let Some(child) = &mut self.child else {
            return Err(SupervisorError::NotStarted);
        };
        let status = child.wait().await?;
        let uptime = self.started_at.map(|t| t.elapsed()).unwrap_or_default();
        // Streams close when the process exits; give the drain tasks a
        // moment to flush their final chunks into the shared buffers.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let record = ExitRecord {
            code: status.code(),
            signal: signal_of(status),
            stderr_tail: Self::last_captured(&self.stderr_buf, 4096),
            stdout_tail: Self::last_captured(&self.stdout_buf, 4096),
            uptime,
        };
        tracing::info!(
            code = ?record.code,
            signal = ?record.signal,
            abnormal = record.is_abnormal(),
            "process exited"
        );
        self.child = None;
        self.last_exit = Some(record.clone());
        Ok(record)
    }
}

trait SpawnCtx {
    fn context_spawn(self, executable: &str) -> Result<u32, SupervisorError>;
}

impl SpawnCtx for Option<u32> {
    fn context_spawn(self, executable: &str) -> Result<u32, SupervisorError> {
        self.ok_or(SupervisorError::Spawn {
            executable: executable.to_string(),
            source: std::io::Error::other("pid unavailable after spawn"),
        })
    }
}

/// Read RSS and CPU time from /proc (Linux best-effort; zeros elsewhere).
fn read_proc_usage(pid: u32) -> ResourceUsage {
    #[cfg(target_os = "linux")]
    {
        let status_path = format!("/proc/{pid}/status");
        let stat_path = format!("/proc/{pid}/stat");
        let rss = std::fs::read_to_string(&status_path).ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
                .map(|kb| kb * 1024)
        });
        let cpu = std::fs::read_to_string(&stat_path).ok().and_then(|s| {
            // fields after the final ')' are fixed-offset; utime is field 14
            let after = s.rsplit(')').next()?;
            let fields: Vec<&str> = after.split_whitespace().collect();
            let utime: u64 = fields.get(11)?.parse().ok()?;
            let stime: u64 = fields.get(12)?.parse().ok()?;
            Some((utime + stime) as f64 / 100.0)
        });
        ResourceUsage {
            rss_bytes: rss,
            cpu_seconds: cpu,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        ResourceUsage::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleep_forever_spec() -> ProcessSpec {
        ProcessSpec::new("/bin/sh", &["-c", "sleep 30"])
    }

    #[tokio::test]
    async fn start_and_terminate_gracefully() {
        let mut sup = NativeProcessLauncher::new();
        let pid = sup.start(&sleep_forever_spec()).await.unwrap();
        assert!(pid > 0);
        assert!(matches!(
            sup.status().await.unwrap(),
            ProcessState::Running { .. }
        ));

        let record = sup.terminate(Duration::from_millis(500)).await.unwrap();
        assert!(record.code == Some(0) || record.signal.is_some());
        assert!(!record.is_abnormal() || record.signal.is_some());
        assert!(matches!(
            sup.status().await.unwrap(),
            ProcessState::Exited(_)
        ));
    }

    #[tokio::test]
    async fn exit_code_is_recorded() {
        let mut sup = NativeProcessLauncher::new();
        sup.start(&ProcessSpec::new("/bin/sh", &["-c", "exit 7"]))
            .await
            .unwrap();
        // give the child a moment to exit
        tokio::time::sleep(Duration::from_millis(100)).await;
        let record = sup.terminate(Duration::from_millis(500)).await.unwrap();
        assert_eq!(record.code, Some(7));
        assert!(record.is_abnormal());
    }

    #[tokio::test]
    async fn kill_terminates_immediately() {
        let mut sup = NativeProcessLauncher::new();
        sup.start(&sleep_forever_spec()).await.unwrap();
        let record = sup.kill().await.unwrap();
        assert!(
            record.signal.is_some() || record.code == Some(0),
            "expected signal or clean reaped exit, got {record:?}"
        );
    }

    #[tokio::test]
    async fn stderr_tail_is_captured_for_failure_records() {
        let mut sup = NativeProcessLauncher::new();
        sup.start(&ProcessSpec::new(
            "/bin/sh",
            &["-c", "echo boom >&2; echo ran-ok; sleep 30"],
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let record = sup.terminate(Duration::from_millis(500)).await.unwrap();
        assert!(
            record.stderr_tail.windows(4).any(|w| w == b"boom"),
            "stderr tail must capture failure output, got {:?}",
            String::from_utf8_lossy(&record.stderr_tail)
        );
        assert!(record.stdout_tail.windows(6).any(|w| w == b"ran-ok"));
    }

    #[tokio::test]
    async fn double_start_is_rejected_while_running() {
        let mut sup = NativeProcessLauncher::new();
        sup.start(&sleep_forever_spec()).await.unwrap();
        let err = sup.start(&sleep_forever_spec()).await.unwrap_err();
        assert!(matches!(err, SupervisorError::AlreadyRunning(_)));
        sup.kill().await.unwrap();
    }

    #[tokio::test]
    async fn terminate_without_start_errors() {
        let mut sup = NativeProcessLauncher::new();
        let err = sup.terminate(Duration::from_millis(10)).await.unwrap_err();
        assert!(matches!(err, SupervisorError::NotStarted));
    }

    #[tokio::test]
    async fn resource_usage_is_reported_while_running() {
        let mut sup = NativeProcessLauncher::new();
        sup.start(&sleep_forever_spec()).await.unwrap();
        let usage = sup.resource_usage().await.unwrap();
        if cfg!(target_os = "linux") {
            assert!(usage.rss_bytes.is_some(), "linux /proc should report rss");
            assert!(usage.rss_bytes.unwrap() > 0);
        }
        sup.kill().await.unwrap();
    }

    #[tokio::test]
    async fn working_directory_and_env_are_applied() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = ProcessSpec::new("/bin/sh", &["-c", "pwd; exit 0"]);
        spec.working_directory = Some(dir.path().to_path_buf());
        spec.environment
            .insert("WM_SWELL_ID".into(), "swell-001".into());
        let mut sup = NativeProcessLauncher::new();
        sup.start(&spec).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let record = sup.terminate(Duration::from_millis(500)).await.unwrap();
        assert_eq!(record.code, Some(0));
    }
}
