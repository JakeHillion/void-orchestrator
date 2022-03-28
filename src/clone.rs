use std::fs::File;
use std::marker::PhantomData;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use libc::{pid_t, syscall, SYS_clone3};
use nix::errno::Errno;
pub use nix::sched::CloneFlags;
use nix::sys::signal::Signal;
use nix::unistd::Pid;

pub struct CloneArgs<'a> {
    pub flags: CloneFlags,
    pub pidfd: Option<&'a mut Option<File>>,
    pub child_tid: Option<&'a mut Option<Pid>>,
    pub parent_tid: Option<&'a mut Option<Pid>>,
    pub exit_signal: Option<Signal>,
    pub stack: Option<&'a mut [u8]>,
    pub set_tid: Option<&'a [Pid]>,
    pub cgroup: Option<&'a File>,
}

#[repr(C)]
struct CloneArgsFfi<'a> {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,

    phantom: PhantomData<&'a ()>,
}

impl<'a: 'b, 'b: 'c, 'c> CloneArgs<'a> {
    pub fn new(flags: CloneFlags) -> CloneArgs<'a> {
        CloneArgs {
            flags,

            pidfd: None,

            child_tid: None,
            parent_tid: None,

            exit_signal: None,

            stack: None,
            set_tid: None,
            cgroup: None,
        }
    }

    fn process(
        &'b mut self,
        pidfd: &mut RawFd,
        child_tid: &mut pid_t,
        parent_tid: &mut pid_t,
    ) -> CloneArgsFfi<'c> {
        CloneArgsFfi {
            flags: self.flags.bits() as u64,
            pidfd: self
                .pidfd
                .as_ref()
                .map(|_| pidfd as *mut RawFd as u64)
                .unwrap_or(0),
            child_tid: self
                .child_tid
                .as_ref()
                .map(|_| child_tid as *mut pid_t as u64)
                .unwrap_or(0),
            parent_tid: self
                .parent_tid
                .as_ref()
                .map(|_| parent_tid as *mut pid_t as u64)
                .unwrap_or(0),
            exit_signal: self.exit_signal.map(|s| s as i32 as u64).unwrap_or(0),
            stack: self
                .stack
                .as_mut()
                .map(|s| s.as_mut_ptr() as u64)
                .unwrap_or(0),
            stack_size: self.stack.as_ref().map(|s| s.len() as u64).unwrap_or(0),
            tls: 0,
            set_tid: self
                .set_tid
                .as_ref()
                .map(|s| s.as_ptr() as u64)
                .unwrap_or(0),
            set_tid_size: self.set_tid.as_ref().map(|s| s.len() as u64).unwrap_or(0),
            cgroup: self.cgroup.map(|c| c.as_raw_fd() as u64).unwrap_or(0),

            phantom: PhantomData,
        }
    }

    unsafe fn finalise(&mut self, pidfd: RawFd, child_tid: pid_t, parent_tid: pid_t) {
        if let Some(r) = &mut self.pidfd {
            **r = Some(File::from_raw_fd(pidfd));
        }
        if let Some(r) = &mut self.child_tid {
            **r = Some(Pid::from_raw(child_tid));
        }
        if let Some(r) = &mut self.parent_tid {
            **r = Some(Pid::from_raw(parent_tid));
        }
    }
}

pub fn clone3(mut args: CloneArgs) -> nix::Result<Pid> {
    let mut pidfd: RawFd = 0;
    let mut child_tid: pid_t = 0;
    let mut parent_tid: pid_t = 0;

    let args_ffi: CloneArgsFfi = args.process(&mut pidfd, &mut child_tid, &mut parent_tid);
    let result = unsafe { syscall(SYS_clone3, &args_ffi, std::mem::size_of_val(&args_ffi)) };

    let out = Errno::result(result).map(|p| Pid::from_raw(p as i32))?;

    // SAFETY: requested things have been filled by the kernel so are now valid for their type
    unsafe {
        args.finalise(pidfd, child_tid, parent_tid);
    }

    Ok(out)
}
