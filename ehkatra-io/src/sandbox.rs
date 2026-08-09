//! Subprocess confinement (docs/24 §Sandbox rule).
//!
//! > *All parsing/serialization runs in an isolated subprocess ... memory/CPU/
//! > wall caps, fresh process per document, **IR-only output revalidated
//! > against schema** by the host.*
//!
//! # What is actually enforced, and what is not
//! Claiming a sandbox you do not have is worse than having none, so the list is
//! explicit.
//!
//! **Enforced by the OS:**
//! * *Address-space isolation.* The parser runs in a different process. A
//!   memory-safety bug in it cannot reach the host's workbook, and this is the
//!   property the whole rule exists for.
//! * *Committed-memory cap* — a Windows job object with
//!   `JOB_OBJECT_LIMIT_PROCESS_MEMORY`. An allocation past the cap fails inside
//!   the child rather than taking the machine with it.
//! * *Process-count cap* of 1: the parser cannot spawn anything.
//! * *Kill on close* — `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` means the child
//!   dies with the host even if the host dies badly. No orphans.
//! * *Wall-clock cap*, enforced by the host, which terminates the whole job.
//! * *Fresh process per document*, so nothing carries between files.
//! * *Output cap.* The host reads at most [`MAX_IR_BYTES`]; a child that will
//!   not stop talking is killed.
//!
//! **Not enforced, and stated rather than implied:**
//! * *No syscall filter.* Windows has no seccomp equivalent reachable without a
//!   driver or an AppContainer profile, and DP-S5 forbids anything requiring
//!   installation or elevation. **"No network" is therefore structural, not
//!   enforced**: `ehkatra-parse` links no networking code and the dependency
//!   budget gate (DP-S2) plus the host-isolation grep are what keep it that
//!   way. That is a real weakening of docs/24's seccomp clause and it is filed
//!   as TD-37 rather than glossed.
//! * *CPU-time cap* is approximated by the wall-clock cap. A job CPU limit
//!   exists but fires per-job-lifetime rather than per-process, which is the
//!   wrong shape for a pool.
//!
//! Non-Windows hosts get address-space isolation, the wall cap, the output cap
//! and the fresh process; the job-object limits are Windows-specific and their
//! POSIX equivalents (`setrlimit`, `prctl`) land with the platform port.

use std::io;
use std::process::Child;
use std::time::Duration;

/// Wall-clock cap for one document.
pub const MAX_WALL: Duration = Duration::from_secs(30);

/// Committed-memory cap for the parser process.
///
/// Sized against `usk_csv::limits`: 16,384 columns × 32,767 bytes is a
/// legitimate 512 MB record, so a cap below that would refuse valid files.
/// This is deliberately a bound on *catastrophe*, not a tight budget.
pub const MAX_MEMORY_BYTES: usize = 1 << 30;

/// The most IR the host will read back from a child.
pub const MAX_IR_BYTES: usize = 256 << 20;

/// A confinement applied to one child process. Dropping it kills the child.
pub struct Sandbox {
    #[cfg(windows)]
    job: windows::Job,
}

impl Sandbox {
    /// Confines `child`. Failing to apply the confinement is an **error**, not
    /// a warning: a sandbox that silently degrades to no sandbox is the exact
    /// failure mode docs/24's "no exceptions" is written against.
    pub fn confine(child: &Child) -> io::Result<Sandbox> {
        #[cfg(windows)]
        {
            let job = windows::Job::create(MAX_MEMORY_BYTES)?;
            job.assign(child)?;
            Ok(Sandbox { job })
        }
        #[cfg(not(windows))]
        {
            // Address-space isolation, the wall cap and the output cap still
            // apply; the resource limits do not. Reported by `is_confined` so a
            // caller can refuse to run rather than discover it later.
            let _ = child;
            Ok(Sandbox {})
        }
    }

    /// Whether OS-level resource limits are in force on this platform. A caller
    /// handling genuinely hostile files should refuse to proceed when this is
    /// false rather than assume the docs/24 guarantees hold.
    pub fn is_confined(&self) -> bool {
        cfg!(windows)
    }

    /// Kills the child and everything it managed to start.
    pub fn terminate(&self) {
        #[cfg(windows)]
        self.job.terminate();
    }
}

#[cfg(windows)]
mod windows {
    //! Raw kernel32 bindings.
    //!
    //! Declared by hand rather than through `windows-sys`, which is ~3 crates
    //! against a DP-S2 budget standing at 29/40 for six functions and one
    //! struct. The struct layout is the risky part, so it is written out in
    //! full with its documented field order rather than padded to a guess.

    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    type Handle = *mut c_void;

    const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    /// `JobObjectExtendedLimitInformation`
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic: BasicLimitInformation,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            class: i32,
            info: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    pub struct Job(Handle);

    // The handle is owned exclusively and only ever passed to kernel32, which
    // is thread-safe for job objects.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn create(memory_limit: usize) -> io::Result<Job> {
            // SAFETY: null attributes and a null name are the documented
            // "unnamed job with default security" call.
            let handle = unsafe { CreateJobObjectW(core::ptr::null_mut(), core::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Job(handle);

            let info = ExtendedLimitInformation {
                basic: BasicLimitInformation {
                    limit_flags: JOB_OBJECT_LIMIT_PROCESS_MEMORY
                        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    // The parser parses. It does not start processes.
                    active_process_limit: 1,
                    ..BasicLimitInformation::default()
                },
                process_memory_limit: memory_limit,
                ..ExtendedLimitInformation::default()
            };
            // SAFETY: `info` matches the layout the class expects and outlives
            // the call, which copies it.
            let ok = unsafe {
                SetInformationJobObject(
                    job.0,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&info as *const ExtendedLimitInformation).cast(),
                    core::mem::size_of::<ExtendedLimitInformation>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: the handle is the live child's, borrowed for this call.
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle().cast()) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: terminating an already-dead job is a no-op that returns
            // an error we deliberately ignore — the goal is "nothing survives",
            // and it is already met.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // KILL_ON_JOB_CLOSE means closing the last handle kills the child.
            // SAFETY: the handle is owned and closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
