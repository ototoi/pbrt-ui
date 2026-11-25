//! A spawned service process together with every descendant it starts,
//! stopped as one: a process group on Unix (plus PDEATHSIG on Linux), a
//! kill-on-close Job Object on Windows (closing it when the host exits takes
//! the children down too).
use std::process::{Child, Command};

pub struct ChildGroup {
    pub child: Child,
    #[cfg(windows)]
    job: job::Job,
}

impl ChildGroup {
    pub fn spawn(mut command: Command) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            #[cfg(target_os = "linux")]
            {
                let parent = std::process::id() as libc::pid_t;
                // Only async-signal-safe syscalls run between fork and exec.
                unsafe {
                    command.pre_exec(move || {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        if libc::getppid() != parent {
                            return Err(std::io::Error::other("host exited before service setup"));
                        }
                        Ok(())
                    });
                }
            }
        }
        #[cfg(windows)]
        let job = {
            use std::os::windows::process::CommandExt;
            // The GUI has no console, so each child would open its own.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
            job::Job::new()?
        };
        let child = command.spawn()?;
        #[cfg(windows)]
        // shortcut: the child runs before it joins the job, so a grandchild it
        // starts in that instant escapes; spawn suspended if that ever matters.
        if let Err(error) = job.assign(&child) {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }

    /// Kill the child and its descendants now (also reaps descendants a
    /// finished child left behind).
    pub fn kill(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        self.job.terminate();
    }

    /// Ask the group to stop. Windows has no polite signal for a console-less
    /// child, so it is the same as `kill` there.
    pub fn terminate(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        self.job.terminate();
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    pub struct Job(HANDLE);
    // The handle is only used through the thread-safe Job Object API.
    unsafe impl Send for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Self> {
            // SAFETY: null attributes and name create an anonymous job; the
            // limit struct is plain data sized as the call states.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }

        pub fn assign(&self, child: &std::process::Child) -> std::io::Result<()> {
            // SAFETY: both handles are live for the call.
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: the job handle is live until drop.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closed once; kill-on-close ends any process still in it.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
