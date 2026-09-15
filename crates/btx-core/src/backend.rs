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

/// Where the NVIDIA driver installs its CUDA library on Linux. The DRIVER's
/// library (`libcuda.so.1`), not the toolkit's `libcudart`: the runtime can be
/// installed on a box with no GPU at all, while the driver library only arrives
/// with a driver, and a driver is only installed against a card. WSL2 mounts it
/// from the Windows host under `/usr/lib/wsl/lib` (measured on this project's
/// RTX 3060 box, 2026-09-15). A distro this list does not know is caught by
/// `ldconfig -p` in `cuda_driver_library`.
#[cfg(target_os = "linux")]
const LINUX_CUDA_DRIVER_LIBRARY_PATHS: &[&str] = &[
    "/usr/lib/wsl/lib/libcuda.so.1",
    "/usr/lib/x86_64-linux-gnu/libcuda.so.1",
    "/usr/lib64/libcuda.so.1",
    "/usr/lib/libcuda.so.1",
    "/usr/local/nvidia/lib64/libcuda.so.1",
    "/run/opengl-driver/lib/libcuda.so.1",
];

/// The path `ldconfig -p` lists for the CUDA driver library, if any. Pure.
///
/// A real line, from a WSL2 host with an RTX 3060 (2026-09-15):
///
/// ```text
///     libcuda.so.1 (libc6,x86-64) => /usr/lib/wsl/lib/libcuda.so.1
/// ```
///
/// Only the soname `libcuda.so.1` counts. `libcudart.so.13` is the toolkit
/// runtime and says nothing about a GPU; `libcudadebugger.so.1` is a tool.
pub fn libcuda_from_ldconfig(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        if line.split_whitespace().next()? != "libcuda.so.1" {
            return None;
        }
        let (_, path) = line.split_once("=>")?;
        Some(path.trim().to_string())
    })
}

/// The NVIDIA driver's CUDA library on this host, if the driver is installed.
///
/// `None` is the node app's launch-time reading of "no GPU btxd could ever
/// validate on"; see `node_host_backend` for what that answer decides and what
/// it deliberately does not claim.
pub fn cuda_driver_library() -> Option<std::path::PathBuf> {
    cuda_driver_library_impl()
}

#[cfg(target_os = "windows")]
fn cuda_driver_library_impl() -> Option<std::path::PathBuf> {
    // The display driver installs the CUDA driver API here on every NVIDIA
    // machine, and the CUDA runtime itself loads it from this path. Present on
    // this project's RTX 3060 Windows host (4,714,728 bytes, 2026-09-15).
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let dll = root.join("System32").join("nvcuda.dll");
    dll.is_file().then_some(dll)
}

#[cfg(target_os = "linux")]
fn cuda_driver_library_impl() -> Option<std::path::PathBuf> {
    if let Some(found) = LINUX_CUDA_DRIVER_LIBRARY_PATHS
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
    {
        return Some(found);
    }
    // A distro the list does not know: ask the loader cache. ldconfig lives in
    // /sbin, which a desktop session's PATH may not carry, so the absolute
    // paths come first; the first ldconfig that RUNS gives the answer.
    for bin in ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"] {
        if let Ok(out) = std::process::Command::new(bin).arg("-p").output() {
            return libcuda_from_ldconfig(&String::from_utf8_lossy(&out.stdout))
                .map(std::path::PathBuf::from);
        }
    }
    None
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn cuda_driver_library_impl() -> Option<std::path::PathBuf> {
    // macOS has had no CUDA driver since 10.13, and the node app runs Metal
    // there. Nothing to look for.
    None
}

/// The backend the NODE app launches btxd with on this host.
///
/// * macOS/aarch64 → `Metal`, at compile time: Apple Silicon self-qualifies.
/// * everywhere else → `Cuda` when the NVIDIA driver's CUDA library is
///   present, `Cpu` when it is not.
///
/// What the answer decides, and what it does NOT claim. Since 2026-09-15
/// `node::build_node_command` splits non-Metal hosts on it: `Cuda` stays in
/// explicit consensus mode and `Cpu` launches as a keyless trusted mirror
/// (docs/decisions/2026-09-15-keyless-cpu-hosts-are-trusted-mirrors.md). So
/// `Cuda` here means "a GPU exists that btxd COULD qualify", not "the card
/// qualifies": a Pascal or Turing card carries the same driver and fails the
/// startup canary, and such a host still takes the consensus path and stalls
/// below the Epoch-A height. Only btxd's own verdict after start
/// (`node::node_rc_status`) knows the difference, and routing on that is the
/// refinement the decision names.
///
/// Why not the miner's probe. `detect_backend` asks `btx-matmul-backend-info`,
/// which applies the matmul library's own device floor and would read a Pascal
/// card as Cpu. The node packages do not ship that tool: `BUNDLED_NODE_BINARIES`
/// lists it as best-effort and `apps/node/scripts/stage-node-pkg-linux-source.sh`
/// copies `btxd` and `btx-cli` only. The driver library is the signal every
/// install already has, with no new binary and no new dependency.
///
/// The `BTX_MATMUL_BACKEND` env this becomes steers MINING, which the node app
/// never does; btxd chooses its RC validation provider by itself. Measured
/// 2026-09-15 on this project's mainnet signer, a btxd this app launched with
/// `BTX_MATMUL_BACKEND=cpu` in its environment: its debug.log reads
/// `provider=cuda_rc_exact_fused_extract ready=1`. The backend matters here for
/// the MODE, never for the device.
///
/// Says which it chose, and why, on stderr: a wrong classification is a wrong
/// consensus posture, and the log is where a maintainer looks first.
pub fn node_host_backend() -> Backend {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Backend::Metal
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        match cuda_driver_library() {
            Some(lib) => {
                eprintln!(
                    "[node] host backend: cuda ({} present), consensus mode on a degraded-start engine",
                    lib.display()
                );
                Backend::Cuda
            }
            None => {
                eprintln!(
                    "[node] host backend: cpu (no NVIDIA driver library found), trusted mirror on a degraded-start engine"
                );
                Backend::Cpu
            }
        }
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

#[cfg(test)]
mod host_backend_tests {
    use super::*;

    /// Verbatim `ldconfig -p` lines from this project's RTX 3060 box (WSL2,
    /// 2026-09-15). Only the driver's soname counts: the toolkit runtime and
    /// the debugger library are present on machines with no GPU at all.
    const LDCONFIG_WITH_DRIVER: &str = "\
\tlibnvidia-ml.so.1 (libc6,x86-64) => /usr/lib/wsl/lib/libnvidia-ml.so.1
\tlibcudart.so.13 (libc6,x86-64) => /usr/local/cuda/targets/x86_64-linux/lib/libcudart.so.13
\tlibcudadebugger.so.1 (libc6,x86-64) => /usr/lib/wsl/lib/libcudadebugger.so.1
\tlibcuda.so.1 (libc6,x86-64) => /usr/lib/wsl/lib/libcuda.so.1
";

    #[test]
    fn the_driver_library_is_read_from_the_loader_cache() {
        assert_eq!(
            libcuda_from_ldconfig(LDCONFIG_WITH_DRIVER).as_deref(),
            Some("/usr/lib/wsl/lib/libcuda.so.1")
        );
    }

    #[test]
    fn the_toolkit_runtime_alone_is_not_a_driver() {
        // A box with CUDA installed for development and no NVIDIA card has
        // libcudart and no libcuda. Counting the runtime would put such a host
        // in consensus mode, where it stalls; that is the wrong direction for
        // this signal to be wrong in.
        let runtime_only = "\
\tlibcudart.so.13 (libc6,x86-64) => /usr/local/cuda/targets/x86_64-linux/lib/libcudart.so.13
\tlibcudadebugger.so.1 (libc6,x86-64) => /usr/lib/wsl/lib/libcudadebugger.so.1
";
        assert_eq!(libcuda_from_ldconfig(runtime_only), None);
        assert_eq!(libcuda_from_ldconfig(""), None);
        // The soname is the first token, not a substring anywhere on the line.
        assert_eq!(
            libcuda_from_ldconfig("\tlibfoo.so.1 (libc6,x86-64) => /opt/libcuda.so.1/x\n"),
            None
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn apple_silicon_is_metal_without_asking() {
        assert_eq!(node_host_backend(), Backend::Metal);
    }

    /// On a PC the answer is whatever the driver check says, and never Metal.
    /// Impure by nature (it reads this machine), so it asserts consistency
    /// between the two public functions rather than a fixed value.
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn a_pc_is_never_metal_and_follows_its_driver() {
        let backend = node_host_backend();
        assert_ne!(backend, Backend::Metal);
        assert_eq!(backend == Backend::Cuda, cuda_driver_library().is_some());
    }
}
