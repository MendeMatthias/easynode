use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::process::Output;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Metal,
    Cuda,
}

impl Backend {
    pub fn as_env(&self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Metal => "metal",
            Backend::Cuda => "cuda",
        }
    }

    /// Parse a backend name as emitted by `btx-matmul-backend-info`
    /// (`matmul::backend::ToString`): `"cpu"`, `"metal"`, or `"cuda"`. The tool
    /// canonicalises `mlx` → `metal`, so we only ever see these three strings,
    /// but we accept `mlx` defensively. Unknown names map to `Cpu`.
    fn from_name(name: &str) -> Backend {
        match name {
            "metal" | "mlx" => Backend::Metal,
            "cuda" => Backend::Cuda,
            _ => Backend::Cpu,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackendStatus {
    pub selected: Backend,
    pub gpu_available: bool,
    pub reason: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[String],
        envs: &[(String, String)],
    ) -> std::io::Result<Output>;
}

/// Real `CommandRunner` backed by `std::process::Command`. Blocking; callers in
/// async contexts should wrap invocations in `tokio::task::spawn_blocking`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        envs: &[(String, String)],
    ) -> std::io::Result<Output> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        // Don't flash a console window on Windows for the backend-info probe.
        // Compiled out on macOS, so the Metal path is unchanged.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd.output()
    }
}

/// Probe `btx-matmul-backend-info --backend <pref> --json` and decide which
/// matmul backend EasyBTX should run with.
///
/// The tool emits a top-level object (see
/// `btx-main/src/btx-matmul-backend-info.cpp` and `backend_capabilities.cpp`)
/// shaped like:
///
/// ```json
/// {
///   "requested_input": "metal",
///   "requested_known": true,
///   "requested_backend": "metal",
///   "active_backend": "metal",
///   "selection_reason": "requested_backend_available",
///   "capabilities": {
///     "cpu":   { "compiled": true,  "available": true,  "reason": "always_available" },
///     "metal": { "compiled": true,  "available": true,  "reason": "..." },
///     "cuda":  { "compiled": false, "available": false, "reason": "disabled_by_build" }
///   },
///   "metal_runtime": { ... },
///   "cuda_runtime":  { ... }
/// }
/// ```
///
/// The tool has ALREADY resolved the effective backend in `active_backend`
/// (applying the same "requested → available? → fallback to CPU" logic we'd
/// otherwise duplicate), with the human-readable rationale in
/// `selection_reason`. So we trust `active_backend` first. If, for some reason,
/// `active_backend` is missing, we fall back to reading
/// `capabilities.<preferred>.available` at its real nested location.
pub fn detect_backend(
    runner: &dyn CommandRunner,
    info_bin: &str,
    preferred: Backend,
) -> AppResult<BackendStatus> {
    if preferred == Backend::Cpu {
        return Ok(BackendStatus {
            selected: Backend::Cpu,
            gpu_available: false,
            reason: "cpu_requested".into(),
        });
    }
    // NOTE: `btx-matmul-backend-info` ALWAYS emits its report as JSON on stdout;
    // it has NO `--json` flag. Earlier code passed `--json`, which this build
    // rejects with "unknown argument" + a non-zero exit and EMPTY stdout — that
    // made detection error out and silently fall back to CPU on Apple Silicon
    // even though Metal was available. Pass only `--backend <pref>`.
    let args = vec!["--backend".to_string(), preferred.as_env().to_string()];
    let out = runner
        .run(info_bin, &args, &[])
        .map_err(|e| AppError::Process(e.to_string()))?;
    // Be tolerant of a tool that writes its report to stderr instead of stdout
    // (some builds do): prefer stdout, fall back to stderr if stdout is empty.
    let raw: &[u8] = if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    };
    let v: Value = serde_json::from_slice(raw).map_err(|e| AppError::Decode(e.to_string()))?;

    // 1. Prefer the tool's own resolved `active_backend` (top-level string).
    if let Some(active) = v.get("active_backend").and_then(|b| b.as_str()) {
        let selected = Backend::from_name(active);
        let reason = v
            .get("selection_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("active_backend")
            .to_string();
        let gpu_available = selected != Backend::Cpu;
        return Ok(BackendStatus {
            selected,
            gpu_available,
            reason,
        });
    }

    // 2. Fallback: read the requested backend's availability under
    //    `capabilities.<backend>` (nested object, NOT top level).
    let key = preferred.as_env(); // "metal" or "cuda"
    let cap = v
        .get("capabilities")
        .and_then(|c| c.get(key))
        .cloned()
        .unwrap_or(Value::Null);
    let available = cap
        .get("available")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let reason = cap
        .get("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("unknown")
        .to_string();
    if available {
        Ok(BackendStatus {
            selected: preferred,
            gpu_available: true,
            reason,
        })
    } else {
        Ok(BackendStatus {
            selected: Backend::Cpu,
            gpu_available: false,
            reason,
        })
    }
}

/// Apply the user's "Force Metal" override (Settings) to a probe result. When on,
/// we ignore the probe's verdict and select Metal regardless — btxd still runs its
/// OWN final probe, so this is a safe "let me opt back in" escape hatch for
/// machines where our helper probe misfires (e.g. a code-signing SIGKILL of the
/// helper on a newer Apple chip) yet the GPU genuinely works. Pure → unit-tested.
pub fn resolve_with_override(detected: BackendStatus, force_metal: bool) -> BackendStatus {
    if force_metal {
        BackendStatus {
            selected: Backend::Metal,
            gpu_available: true,
            reason: "forced_by_user".to_string(),
        }
    } else {
        detected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    /// Build a fake `ExitStatus` carrying the given process exit code, on either
    /// platform. unix `ExitStatusExt::from_raw` takes a raw wait status (code in
    /// bits 8..15, so callers pass `code << 8`); windows `ExitStatusExt::from_raw`
    /// takes the exit code directly. This helper hides that difference so the mock
    /// runners below compile + behave the same on macOS, Linux, and Windows.
    fn fake_status(code: u32) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw((code << 8) as i32)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code)
        }
    }

    #[test]
    fn override_forces_metal_over_a_cpu_probe_result() {
        // The exact M4 case: the probe fell back to CPU, but the user forces Metal.
        let detected = BackendStatus {
            selected: Backend::Cpu,
            gpu_available: false,
            reason: "detection_failed: probe killed".into(),
        };
        let s = resolve_with_override(detected, true);
        assert_eq!(s.selected, Backend::Metal);
        assert!(s.gpu_available);
        assert_eq!(s.reason, "forced_by_user");
    }

    #[test]
    fn override_off_passes_the_probe_result_through_unchanged() {
        let detected = BackendStatus {
            selected: Backend::Cpu,
            gpu_available: false,
            reason: "no_supported_device".into(),
        };
        assert_eq!(resolve_with_override(detected.clone(), false), detected);
    }

    struct FakeRunner {
        stdout: Vec<u8>,
    }
    impl CommandRunner for FakeRunner {
        fn run(&self, _p: &str, _a: &[String], _e: &[(String, String)]) -> std::io::Result<Output> {
            Ok(Output {
                status: fake_status(0),
                stdout: self.stdout.clone(),
                stderr: vec![],
            })
        }
    }

    /// Runner that records the args it was invoked with, so we can assert the
    /// exact CLI we hand to `btx-matmul-backend-info`.
    struct ArgRecordingRunner {
        stdout: Vec<u8>,
        seen_args: std::sync::Mutex<Vec<String>>,
    }
    impl CommandRunner for ArgRecordingRunner {
        fn run(&self, _p: &str, a: &[String], _e: &[(String, String)]) -> std::io::Result<Output> {
            *self.seen_args.lock().unwrap() = a.to_vec();
            Ok(Output {
                status: fake_status(0),
                stdout: self.stdout.clone(),
                stderr: vec![],
            })
        }
    }

    /// Runner mimicking the real v0.30.1 tool: it does NOT understand `--json`
    /// (exits non-zero with empty stdout when it sees it), but emits JSON on
    /// stdout otherwise. We verify detect_backend never passes `--json`.
    struct JsonFlagIntolerantRunner {
        stdout: Vec<u8>,
    }
    impl CommandRunner for JsonFlagIntolerantRunner {
        fn run(&self, _p: &str, a: &[String], _e: &[(String, String)]) -> std::io::Result<Output> {
            if a.iter().any(|x| x == "--json") {
                // Real tool: "error: unknown argument: --json", exit 1, no stdout.
                return Ok(Output {
                    status: fake_status(1),
                    stdout: vec![],
                    stderr: b"error: unknown argument: --json".to_vec(),
                });
            }
            Ok(Output {
                status: fake_status(0),
                stdout: self.stdout.clone(),
                stderr: vec![],
            })
        }
    }

    // Realistic top-level payload matching btx-matmul-backend-info's actual
    // output (truncated runtime objects). The fields EasyBTX reads are
    // `active_backend`, `selection_reason`, and `capabilities.<backend>`.

    /// A Metal-capable macOS box: the tool resolves `active_backend: "metal"`.
    const METAL_AVAILABLE: &str = r#"{
      "requested_input": "metal",
      "requested_known": true,
      "requested_backend": "metal",
      "active_backend": "metal",
      "selection_reason": "requested_backend_available",
      "capabilities": {
        "cpu":   { "compiled": true,  "available": true,  "reason": "always_available" },
        "metal": { "compiled": true,  "available": true,  "reason": "metal_device_present" },
        "cuda":  { "compiled": false, "available": false, "reason": "disabled_by_build" }
      },
      "metal_runtime": { "buffer_pool": { "available": true } }
    }"#;

    /// A box where CUDA was requested but no device is present: the tool falls
    /// back to CPU itself (`active_backend: "cpu"`) and explains why.
    const CUDA_UNAVAILABLE: &str = r#"{
      "requested_input": "cuda",
      "requested_known": true,
      "requested_backend": "cuda",
      "active_backend": "cpu",
      "selection_reason": "cuda_unavailable_fallback_to_cpu:no_supported_device",
      "capabilities": {
        "cpu":   { "compiled": true,  "available": true,  "reason": "always_available" },
        "metal": { "compiled": false, "available": false, "reason": "disabled_by_build" },
        "cuda":  { "compiled": true,  "available": false, "reason": "no_supported_device" }
      },
      "cuda_runtime": { "available": false, "reason": "no_supported_device" }
    }"#;

    #[test]
    fn selects_metal_when_active_backend_is_metal() {
        let r = FakeRunner {
            stdout: METAL_AVAILABLE.as_bytes().to_vec(),
        };
        let s = detect_backend(&r, "btx-matmul-backend-info", Backend::Metal).unwrap();
        assert_eq!(
            s,
            BackendStatus {
                selected: Backend::Metal,
                gpu_available: true,
                reason: "requested_backend_available".into(),
            }
        );
    }

    #[test]
    fn falls_back_to_cpu_when_active_backend_is_cpu() {
        let r = FakeRunner {
            stdout: CUDA_UNAVAILABLE.as_bytes().to_vec(),
        };
        let s = detect_backend(&r, "btx-matmul-backend-info", Backend::Cuda).unwrap();
        assert_eq!(s.selected, Backend::Cpu);
        assert!(!s.gpu_available);
        // The tool's own selection_reason is surfaced verbatim.
        assert_eq!(
            s.reason,
            "cuda_unavailable_fallback_to_cpu:no_supported_device"
        );
    }

    /// Regression: detect_backend must NOT pass `--json` (the real v0.30.1
    /// `btx-matmul-backend-info` rejects it). It should pass only
    /// `--backend <pref>`.
    #[test]
    fn does_not_pass_json_flag() {
        let r = ArgRecordingRunner {
            stdout: METAL_AVAILABLE.as_bytes().to_vec(),
            seen_args: std::sync::Mutex::new(Vec::new()),
        };
        detect_backend(&r, "btx-matmul-backend-info", Backend::Metal).unwrap();
        let args = r.seen_args.lock().unwrap().clone();
        assert_eq!(
            args,
            vec!["--backend".to_string(), "metal".to_string()],
            "must pass only --backend <pref>, never --json; got {args:?}"
        );
    }

    /// Regression for the live "Backend: cpu on Apple Silicon" bug: with a tool
    /// that errors on `--json` (empty stdout, exit 1) but emits JSON otherwise,
    /// detection must still resolve Metal — proving we dropped `--json`.
    #[test]
    fn resolves_metal_against_json_intolerant_tool() {
        let r = JsonFlagIntolerantRunner {
            stdout: METAL_AVAILABLE.as_bytes().to_vec(),
        };
        let s = detect_backend(&r, "btx-matmul-backend-info", Backend::Metal).unwrap();
        assert_eq!(s.selected, Backend::Metal);
        assert!(s.gpu_available);
    }

    /// Fallback path: if the tool ever omits `active_backend`, we read
    /// `capabilities.<backend>.available` at its real nested location.
    #[test]
    fn reads_nested_capabilities_when_active_backend_absent() {
        let payload = r#"{
          "requested_backend": "metal",
          "capabilities": {
            "metal": { "compiled": true, "available": true, "reason": "metal_device_present" }
          }
        }"#;
        let r = FakeRunner {
            stdout: payload.as_bytes().to_vec(),
        };
        let s = detect_backend(&r, "btx-matmul-backend-info", Backend::Metal).unwrap();
        assert_eq!(s.selected, Backend::Metal);
        assert!(s.gpu_available);
        assert_eq!(s.reason, "metal_device_present");
    }
}
