//! KBEngine bots process startup (WM-0204).
//!
//! The Supervisor launches one KBEngine bots process per Backend instance
//! with the Bridge configuration injected as environment variables
//! (docs/kbengine.md "KBEngine Bridge Lifecycle" step 3; the first version
//! uses the native launcher — docs/kbengine.md "Target Deployment Shape").
//! The Python Bridge inside the process reads exactly these variables at
//! `onInit` (`engine-adapters/kbengine/bridge/bootstrap.py`
//! `BootstrapConfig.ENV_*` — the names here are the single source of truth
//! for both sides and must never drift).
//!
//! What this module does *not* do: own the IPC listener the Bridge connects
//! to (Supervisor-side session handling rides with the Backend service
//! work), and restart automatically (S6: a restart is a new process with a
//! fresh `onInit`; the recorded [`RestartPolicy`] stays inert unless the
//! swell failure policy explicitly allows it — plan §7.3).

use std::collections::HashMap;

use crate::{ProcessSpec, RestartPolicy, StreamSink};

/// Local IPC address the Bridge connects to (`unix:<path>`, a bare socket
/// path, or `tcp:<host>:<port>`; parsed by `bridge.ipc_client.parse_address`).
pub const BRIDGE_IPC_ENV: &str = "WAVEMAKER_BRIDGE_IPC";
/// Shared secret for the IPC session (checked by the Supervisor listener).
pub const BRIDGE_TOKEN_ENV: &str = "WAVEMAKER_BRIDGE_TOKEN";
/// Swell this Backend instance serves.
pub const SWELL_ID_ENV: &str = "WAVEMAKER_SWELL_ID";
/// Backend instance id announced in the Bridge handshake.
pub const BACKEND_ID_ENV: &str = "WAVEMAKER_BACKEND_ID";
/// Backend generation this process was launched for (decimal u64; 0 =
/// unset). The handshake ack's generation stays authoritative — this is the
/// launch hint the Supervisor side can cross-check against its own record.
pub const BACKEND_GENERATION_ENV: &str = "WAVEMAKER_BACKEND_GENERATION";

/// Everything needed to launch one KBEngine bots process (plan §7.2
/// ProcessSpec specialised for the KBEngine adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotsProcessSpec {
    /// Path to the KBEngine bots binary (e.g. the `bots` tool built from the
    /// KBEngine tree).
    pub bots_executable: String,
    /// Extra arguments passed to the bots binary (config paths, flags).
    pub bots_args: Vec<String>,
    /// Working directory for the engine process (script lookup root).
    pub working_directory: Option<std::path::PathBuf>,
    /// IPC address injected as [`BRIDGE_IPC_ENV`].
    pub bridge_ipc: String,
    /// IPC token injected as [`BRIDGE_TOKEN_ENV`].
    pub bridge_token: String,
    /// Injected as [`SWELL_ID_ENV`].
    pub swell_id: String,
    /// Injected as [`BACKEND_ID_ENV`].
    pub backend_id: String,
    /// Injected as [`BACKEND_GENERATION_ENV`] (0 = unset).
    pub backend_generation: u64,
    /// Additional environment for the child (PATH tweaks, engine vars).
    pub extra_environment: HashMap<String, String>,
    pub shutdown_grace_period_ms: u64,
}

impl BotsProcessSpec {
    /// Minimal spec: bots binary plus the Bridge variables.
    pub fn new(bots_executable: impl Into<String>, bridge_ipc: impl Into<String>) -> Self {
        Self {
            bots_executable: bots_executable.into(),
            bots_args: Vec::new(),
            working_directory: None,
            bridge_ipc: bridge_ipc.into(),
            bridge_token: String::new(),
            swell_id: String::new(),
            backend_id: String::new(),
            backend_generation: 0,
            extra_environment: HashMap::new(),
            shutdown_grace_period_ms: 5000,
        }
    }

    /// The exact environment the Bridge bootstrap reads at `onInit`.
    pub fn bridge_environment(&self) -> HashMap<String, String> {
        let mut env = HashMap::with_capacity(5 + self.extra_environment.len());
        env.insert(BRIDGE_IPC_ENV.to_string(), self.bridge_ipc.clone());
        env.insert(BRIDGE_TOKEN_ENV.to_string(), self.bridge_token.clone());
        env.insert(SWELL_ID_ENV.to_string(), self.swell_id.clone());
        env.insert(BACKEND_ID_ENV.to_string(), self.backend_id.clone());
        env.insert(
            BACKEND_GENERATION_ENV.to_string(),
            self.backend_generation.to_string(),
        );
        env.extend(self.extra_environment.clone());
        env
    }

    /// Generalise into the launcher-agnostic [`ProcessSpec`]: captured
    /// streams (exit records keep a log tail for failures), never restart.
    pub fn to_process_spec(&self) -> ProcessSpec {
        ProcessSpec {
            executable: self.bots_executable.clone(),
            args: self.bots_args.clone(),
            working_directory: self.working_directory.clone(),
            environment: self.bridge_environment(),
            stdout: StreamSink::Capture,
            stderr: StreamSink::Capture,
            restart_policy: RestartPolicy::Never,
            shutdown_grace_period_ms: self.shutdown_grace_period_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativeProcessLauncher, ProcessState, ProcessSupervisor};
    use std::time::Duration;

    #[test]
    fn env_names_match_the_python_bridge_contract() {
        // engine-adapters/kbengine/bridge/bootstrap.py BootstrapConfig.ENV_*:
        // these literals are the cross-language wire contract.
        assert_eq!(BRIDGE_IPC_ENV, "WAVEMAKER_BRIDGE_IPC");
        assert_eq!(BRIDGE_TOKEN_ENV, "WAVEMAKER_BRIDGE_TOKEN");
        assert_eq!(SWELL_ID_ENV, "WAVEMAKER_SWELL_ID");
        assert_eq!(BACKEND_ID_ENV, "WAVEMAKER_BACKEND_ID");
        assert_eq!(BACKEND_GENERATION_ENV, "WAVEMAKER_BACKEND_GENERATION");
    }

    #[test]
    fn bridge_environment_carries_the_four_variables_plus_extras() {
        let mut spec = BotsProcessSpec::new("/opt/kbe/bots", "unix:/tmp/wm.sock");
        spec.bridge_token = "tok".into();
        spec.swell_id = "sw-1".into();
        spec.backend_id = "be-9".into();
        spec.backend_generation = 12;
        spec.extra_environment
            .insert("KBE_ROOT".to_string(), "/opt/kbe".to_string());
        let env = spec.bridge_environment();
        assert_eq!(env.get(BRIDGE_IPC_ENV).unwrap(), "unix:/tmp/wm.sock");
        assert_eq!(env.get(BRIDGE_TOKEN_ENV).unwrap(), "tok");
        assert_eq!(env.get(SWELL_ID_ENV).unwrap(), "sw-1");
        assert_eq!(env.get(BACKEND_ID_ENV).unwrap(), "be-9");
        assert_eq!(env.get(BACKEND_GENERATION_ENV).unwrap(), "12");
        assert_eq!(env.get("KBE_ROOT").unwrap(), "/opt/kbe");
    }

    #[test]
    fn to_process_spec_generalises_with_capture_and_no_restart() {
        let spec = BotsProcessSpec::new("/opt/kbe/bots", "tcp:127.0.0.1:9000");
        let ps = spec.to_process_spec();
        assert_eq!(ps.executable, "/opt/kbe/bots");
        assert!(ps.args.is_empty());
        assert_eq!(ps.stdout, StreamSink::Capture);
        assert_eq!(ps.stderr, StreamSink::Capture);
        assert_eq!(ps.restart_policy, RestartPolicy::Never);
        assert_eq!(
            ps.environment.get(BRIDGE_IPC_ENV).unwrap(),
            "tcp:127.0.0.1:9000"
        );
    }

    /// The injected configuration must actually reach the child process:
    /// a real spawn echoes the variables back through captured stdout.
    #[tokio::test]
    async fn startup_injects_bridge_env_into_the_child() {
        let mut spec = BotsProcessSpec::new("/bin/sh", "unix:/tmp/wm-e2e.sock");
        spec.bots_args = vec![
            "-c".into(),
            "echo \"$WAVEMAKER_BRIDGE_IPC|$WAVEMAKER_SWELL_ID|$WAVEMAKER_BACKEND_ID\"".into(),
        ];
        spec.swell_id = "sw-42".into();
        spec.backend_id = "be-7".into();

        let mut sup = NativeProcessLauncher::new();
        sup.start(&spec.to_process_spec()).await.unwrap();
        // The child exits on its own; wait for that instead of racing an
        // immediate terminate (a SIGTERM mid-echo would mask the exit code).
        let mut record = None;
        for _ in 0..100 {
            if let ProcessState::Exited(r) = sup.status().await.unwrap() {
                record = Some(r);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let record = record.expect("child exited on its own");
        assert_eq!(record.code, Some(0));
        let out = String::from_utf8_lossy(&record.stdout_tail);
        assert!(
            out.contains("unix:/tmp/wm-e2e.sock|sw-42|be-7"),
            "child saw {out:?}"
        );
    }

    /// A bots process that dies before the handshake is recorded with its
    /// stderr tail (plan §7.3: unexpected exits keep evidence).
    #[tokio::test]
    async fn early_death_is_recorded_with_stderr_tail() {
        let mut spec = BotsProcessSpec::new("/bin/sh", "unix:/tmp/wm-early.sock");
        spec.bots_args = vec!["-c".into(), "echo config-missing >&2; exit 3".into()];
        let mut sup = NativeProcessLauncher::new();
        sup.start(&spec.to_process_spec()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let record = sup.terminate(Duration::from_millis(2000)).await.unwrap();
        assert_eq!(record.code, Some(3));
        assert!(record.is_abnormal());
        let err = String::from_utf8_lossy(&record.stderr_tail);
        assert!(err.contains("config-missing"), "stderr tail: {err:?}");
        assert!(matches!(
            sup.status().await.unwrap(),
            ProcessState::Exited(_)
        ));
    }
}
