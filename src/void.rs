use log::{debug, error};

use crate::clone::{clone3, CloneArgs, CloneFlags};
use crate::{Error, Result};

use std::collections::{HashMap, HashSet};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::{fmt, fs};

use nix::fcntl::{FcntlArg, FdFlag};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::unshare;
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::unistd::{pivot_root, Pid};

use close_fds::CloseFdsBuilder;

pub struct VoidHandle {
    pid: Pid,
}

impl fmt::Display for VoidHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Void{{Pid:{}}}", self.pid)
    }
}

pub struct VoidBuilder {
    mounts: HashMap<PathBuf, PathBuf>,
    fds: HashSet<RawFd>,
}

impl VoidBuilder {
    pub fn new() -> VoidBuilder {
        VoidBuilder {
            mounts: HashMap::new(),
            fds: HashSet::new(),
        }
    }

    pub fn mount<T1: AsRef<Path>, T2: AsRef<Path>>(&mut self, src: T1, dst: T2) -> &mut Self {
        self.mounts.insert(src.as_ref().into(), dst.as_ref().into());
        self
    }

    pub fn keep_fd(&mut self, fd: &impl AsRawFd) -> &mut Self {
        self.fds.insert(fd.as_raw_fd());
        self
    }

    pub fn spawn(&mut self, child_fn: impl FnOnce() -> i32) -> Result<VoidHandle> {
        let mut args = CloneArgs::new(
            CloneFlags::CLONE_NEWCGROUP
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWNET
                | CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWUTS,
        );
        args.exit_signal = Some(Signal::SIGCHLD);

        let child = clone3(args).map_err(|e| Error::Nix {
            msg: "clone3",
            src: e,
        })?;

        if child == Pid::from_raw(0) {
            // ignore SIGHUP
            // safety: safe as ignores the return result of the previous handler
            unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }.map_err(|e| Error::Nix {
                msg: "signal",
                src: e,
            })?;

            let result = {
                self.void_files()?;
                self.void_mount_namespace()?;
                self.void_user_namespace()?; // last to maintain permissions

                Ok::<(), Error>(())
            };

            if let Err(e) = result {
                error!("error preparing void: {}", e);
                std::process::exit(-1)
            } else {
                std::process::exit(child_fn())
            }
        }

        debug!("cloned child: {}", child);

        // Leak the child function's resources in the parent process.
        // This avoids closing files that have been "moved" into the child.
        // It is also an over-approximation, and may cause actual memory leaks.
        // As the spawning process is normally short lived, this shouldn't
        // be a problem.
        std::mem::forget(child_fn);

        Ok(VoidHandle { pid: child })
    }

    // per-namespace void creation
    fn void_files(&self) -> Result<()> {
        let mut closer = CloseFdsBuilder::new();

        let keep: Box<[RawFd]> = self.fds.iter().copied().collect();
        closer.keep_fds(&keep);

        unsafe {
            closer.closefrom(3);
        }

        for fd in keep.as_ref() {
            let mut flags = FdFlag::from_bits_truncate(
                nix::fcntl::fcntl(*fd, FcntlArg::F_GETFD).map_err(|e| Error::Nix {
                    msg: "fcntl",
                    src: e,
                })?,
            );

            flags.remove(FdFlag::FD_CLOEXEC);

            nix::fcntl::fcntl(*fd, FcntlArg::F_SETFD(flags)).map_err(|e| Error::Nix {
                msg: "fcntl",
                src: e,
            })?;
        }

        Ok(())
    }

    fn void_mount_namespace(&self) -> Result<()> {
        // change the propagation type of the old root to private
        mount(
            Option::<&str>::None,
            "/",
            Option::<&str>::None,
            MsFlags::MS_PRIVATE,
            Option::<&str>::None,
        )
        .map_err(|e| Error::Nix {
            msg: "mount",
            src: e,
        })?;

        // create and consume a tmpdir to mount a tmpfs into
        let new_root = tempfile::tempdir()?.into_path();

        // mount a tmpfs as the new root
        mount(
            Some("tmpfs"),
            &new_root,
            Some("tmpfs"),
            MsFlags::empty(),
            Option::<&str>::None,
        )
        .map_err(|e| Error::Nix {
            msg: "mount",
            src: e,
        })?;

        // prepare a subdirectory to pivot the old root into
        let old_root = new_root.join("old_root/");
        debug!("new_root: {:?}; put_old: {:?}", &new_root, &old_root);
        fs::create_dir(&old_root)?;

        // pivot the old root into a subdirectory of the new root
        pivot_root(&new_root, &old_root).map_err(|e| Error::Nix {
            msg: "pivot_root",
            src: e,
        })?;

        let new_root = PathBuf::from("/");
        let old_root = PathBuf::from("/old_root/");

        // chdir after
        std::env::set_current_dir(&new_root)?;

        // mount paths before unmounting old_root
        for (src, dst) in &self.mounts {
            let mut src = old_root.join(src.strip_prefix("/").unwrap_or(src));
            let dst = new_root.join(dst.strip_prefix("/").unwrap_or(dst));

            debug!("mounting `{:?}` as `{:?}`", src, dst);

            // create the target
            let mut src_data = fs::symlink_metadata(&src)?;

            if src_data.is_symlink() {
                let link = fs::read_link(src)?;

                src = old_root.join(link.strip_prefix("/").unwrap_or(&link));
                src_data = fs::metadata(&src)?;
            }

            if src_data.is_dir() {
                fs::create_dir_all(&dst)?;
            } else {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&dst, b"")?;
            }

            // bind mount
            mount(
                Some(&src),
                &dst,
                Option::<&str>::None,
                MsFlags::MS_BIND,
                Option::<&str>::None,
            )
            .map_err(|e| Error::Nix {
                msg: "mount",
                src: e,
            })?;
        }

        // recursively remount the old root as private to avoid shared unmounting
        // the submounts (because MNT_DETACH is recursive)
        mount(
            Option::<&str>::None,
            &old_root,
            Option::<&str>::None,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            Option::<&str>::None,
        )
        .map_err(|e| Error::Nix {
            msg: "mount",
            src: e,
        })?;

        // unmount the old root
        umount2(&old_root, MntFlags::MNT_DETACH).map_err(|e| Error::Nix {
            msg: "umount2",
            src: e,
        })?;

        // delete the old root mount point
        fs::remove_dir(&old_root)?;

        Ok(())
    }

    fn void_user_namespace(&self) -> Result<()> {
        unshare(CloneFlags::CLONE_NEWUSER).map_err(|e| Error::Nix {
            msg: "unshare",
            src: e,
        })?;

        Ok(())
    }
}
