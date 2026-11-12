//! Child processes that die with the engine.

use std::io;
use std::process::{Child, Command};

/// Starts `cmd` so that the OS kills the child if this process dies, however it dies.
/// Every engine-started process (tools, converters, servers, helpers) must go through here
/// instead of `Command::spawn`.
///
/// - Windows: [`init`](crate::init) puts this process in a kill-on-close job object, which
///   every descendant inherits; if that failed, the child is assigned to the job right
///   after it starts.
/// - Linux: the child gets `PR_SET_PDEATHSIG = SIGKILL`. Linux sends it when the *thread*
///   that forked the child exits, so every spawn runs on one spawner thread that lives as
///   long as the process. A child that starts its own children must do the same for them
///   (engine binaries do, through this function).
/// - Other Unix systems: a plain spawn (no OS mechanism; not a supported target yet).
pub fn spawn(cmd: Command) -> io::Result<Child> {
    imp::spawn(cmd)
}

/// Whether descendants of this process die with it without per-child work (Windows job
/// object holding this process). On Linux this is `false` and [`spawn`] does the work per
/// child.
pub fn contained() -> bool {
    imp::contained()
}

pub(crate) fn contain() -> bool {
    imp::contain()
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Mutex, OnceLock};

    type Job = (Command, Sender<io::Result<Child>>);

    fn spawner() -> io::Result<Sender<Job>> {
        static SPAWNER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
        if let Some(s) = SPAWNER.get() {
            return Ok(s.lock().unwrap_or_else(|p| p.into_inner()).clone());
        }
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("marionette-spawner".into())
            .spawn(move || {
                for (mut cmd, reply) in rx {
                    let _ = reply.send(spawn_here(&mut cmd));
                }
            })?;
        let s = SPAWNER.get_or_init(|| Mutex::new(tx));
        Ok(s.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    fn spawn_here(cmd: &mut Command) -> io::Result<Child> {
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs in the forked child before exec and only calls
        // async-signal-safe functions (prctl, getppid, _exit).
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // The parent died between fork and prctl: nobody would ever kill us.
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
        cmd.spawn()
    }

    pub fn spawn(cmd: Command) -> io::Result<Child> {
        let (tx, rx) = channel();
        spawner()?
            .send((cmd, tx))
            .map_err(|_| io::Error::other("the spawner thread is gone"))?;
        rx.recv()
            .map_err(|_| io::Error::other("the spawner thread is gone"))?
    }

    pub fn contained() -> bool {
        false
    }

    pub fn contain() -> bool {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Child, Command};
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// The job handle (as an integer: raw handles are not `Sync`) and whether this process
    /// is in it. The handle is never closed: the OS closes it when the process ends, which
    /// is what kills the job's processes.
    struct Job {
        handle: usize,
        holds_self: bool,
    }

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid arguments; the info struct is fully
            // initialized (zeroed, then the one flag set).
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return None;
                }
                let holds_self = AssignProcessToJobObject(handle, GetCurrentProcess()) != 0;
                Some(Job {
                    handle: handle as usize,
                    holds_self,
                })
            }
        })
        .as_ref()
    }

    pub fn spawn(mut cmd: Command) -> io::Result<Child> {
        let Some(j) = job() else {
            return Err(io::Error::other(
                "cannot create a job object, so the child would outlive a crash; refusing to start it",
            ));
        };
        let child = cmd.spawn()?;
        if !j.holds_self {
            // SAFETY: both handles are valid for the duration of the call.
            let ok = unsafe { AssignProcessToJobObject(j.handle as _, child.as_raw_handle() as _) };
            if ok == 0 {
                let err = io::Error::last_os_error();
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
        Ok(child)
    }

    pub fn contained() -> bool {
        job().is_some_and(|j| j.holds_self)
    }

    pub fn contain() -> bool {
        contained()
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use std::io;
    use std::process::{Child, Command};

    pub fn spawn(mut cmd: Command) -> io::Result<Child> {
        cmd.spawn()
    }
    pub fn contained() -> bool {
        false
    }
    pub fn contain() -> bool {
        false
    }
}
