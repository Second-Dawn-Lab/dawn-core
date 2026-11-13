//! The built-in backend: `llama-server` (llama.cpp, MIT) started on demand on a free port of this computer, stopped when
//! it is idle or when the app quits. It is never started at app start.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::client::Client;
use crate::endpoint::Endpoint;
use crate::error::{LlmError, Result};

/// What to run.
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarSpec {
    pub exe: PathBuf,
    pub model: PathBuf,
    pub threads: u32,
    /// Context size in tokens; a sentence plus its draft needs little, a document paragraph more.
    pub context: u32,
}

pub struct Sidecar {
    child: Child,
    pub client: Client,
}

/// The command line, without running it (tested).
pub fn arguments(spec: &SidecarSpec, port: u16) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        spec.model.to_string_lossy().into_owned(),
        // This computer only, never "0.0.0.0".
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        spec.context.to_string(),
        // One request at a time: the memory of one slot, not four.
        "-np".into(),
        "1".into(),
        "--no-webui".into(),
        "--jinja".into(),
    ];
    if spec.threads > 0 {
        args.push("-t".into());
        args.push(spec.threads.to_string());
    }
    args
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| LlmError::Server(e.to_string()))?;
    listener.local_addr().map(|a| a.port()).map_err(|e| LlmError::Server(e.to_string()))
}

impl Sidecar {
    /// Start the server and wait until the model is loaded (`/health`), at most `timeout`.
    pub fn start(spec: &SidecarSpec, timeout: Duration) -> Result<Self> {
        if !spec.exe.exists() {
            return Err(LlmError::Unavailable("the Quality mode runtime is not installed".into()));
        }
        if !spec.model.exists() {
            return Err(LlmError::Unavailable("the language model is not installed".into()));
        }
        let port = free_port()?;
        let mut command = Command::new(&spec.exe);
        command.args(arguments(spec, port)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // The runtime's DLLs sit next to it.
        if let Some(dir) = spec.exe.parent() {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = command.spawn().map_err(|e| LlmError::Server(format!("could not start the runtime: {e}")))?;
        bind_to_app(&child);
        let mut sidecar = Self { child, client: Client::new(Endpoint::local(port), "") };
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = sidecar.child.try_wait() {
                return Err(LlmError::Server(format!("the runtime stopped while loading the model ({status})")));
            }
            if sidecar.client.is_ready() {
                return Ok(sidecar);
            }
            if started.elapsed() > timeout {
                return Err(LlmError::Server("loading the model took too long".into()));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The runtime must never outlive Owl-L, also when Owl-L crashes or is ended from the Task Manager: the child is put in a
/// job object that kills its processes when the last handle to it closes, which happens when Owl-L's process ends.
#[cfg(windows)]
fn bind_to_app(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // The job lives as long as the app (never closed on purpose); its handle is stored as a number to be `Send`.
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    let job = JOB.get_or_init(|| {
        // SAFETY: plain Win32 calls with valid arguments; the info struct is fully initialized and correctly sized.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).ok()?;
            Some(job.0 as isize)
        }
    });
    if let Some(job) = *job {
        // SAFETY: both handles are valid; a failure (the process is already in another job) only loses the extra safety.
        unsafe {
            let _ = AssignProcessToJobObject(HANDLE(job as *mut _), HANDLE(child.as_raw_handle()));
        }
    }
}

#[cfg(not(windows))]
fn bind_to_app(_child: &Child) {}

/// Is `path` an existing file? (used by the settings page to show "runtime installed")
pub fn exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_listens_on_this_computer_only_with_one_slot_and_no_web_page() {
        let spec = SidecarSpec { exe: "llama-server.exe".into(), model: "m.gguf".into(), threads: 6, context: 4096 };
        let args = arguments(&spec, 50123);
        let pos = |a: &str| args.iter().position(|x| x == a).unwrap();
        assert_eq!(args[pos("--host") + 1], "127.0.0.1");
        assert_eq!(args[pos("--port") + 1], "50123");
        assert_eq!(args[pos("-np") + 1], "1");
        assert!(args.contains(&"--no-webui".to_string()));
        assert_eq!(args[pos("-t") + 1], "6");
        assert!(!args.iter().any(|a| a == "0.0.0.0"));
        let auto = arguments(&SidecarSpec { threads: 0, ..spec }, 1);
        assert!(!auto.contains(&"-t".to_string()), "0 threads = let the runtime decide");
    }

    #[test]
    fn a_missing_runtime_or_model_is_a_clear_error() {
        let spec = SidecarSpec { exe: "does-not-exist.exe".into(), model: "m.gguf".into(), threads: 0, context: 1024 };
        assert!(matches!(Sidecar::start(&spec, Duration::from_secs(1)), Err(LlmError::Unavailable(_))));
    }

    #[test]
    fn free_ports_are_found() {
        assert!(free_port().unwrap() > 1023);
    }
}
