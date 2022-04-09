use log::{debug, error};

use crate::clone::{clone3, CloneArgs, CloneFlags};
use crate::{Error, Result};

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::unshare;
use nix::sys::signal::Signal;
use nix::unistd::{pivot_root, Pid};

use close_fds::CloseFdsBuilder;

pub struct VoidHandle {}

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

        Ok(VoidHandle {})
    }

    // per-namespace void creation
    fn void_files(&self) -> Result<()> {
        let mut closer = CloseFdsBuilder::new();

        let keep: Box<[RawFd]> = self.fds.iter().copied().collect();
        closer.keep_fds(&keep);

        unsafe {
            closer.closefrom(3);
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
        let put_old = new_root.join("old_root/");
        debug!("new_root: {:?}; put_old: {:?}", &new_root, &put_old);
        fs::create_dir(&put_old)?;

        // pivot the old root into a subdirectory of the new root
        pivot_root(&new_root, &put_old).map_err(|e| Error::Nix {
            msg: "pivot_root",
            src: e,
        })?;

        // chdir after
        std::env::set_current_dir("/")?;

        // mount paths before unmounting old_root
        for (src, dst) in &self.mounts {
            let src = PathBuf::from("/old_root/").join(src.strip_prefix("/").unwrap_or(src));
            let dst = PathBuf::from("/").join(dst);

            debug!("mounting `{:?}` as `{:?}`", src, dst);

            // create the target
            let src_data = fs::metadata(&src)?;
            if src_data.is_dir() {
                fs::create_dir_all(&dst)?;
            } else {
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

        // unmount the old root
        umount2("/old_root/", MntFlags::MNT_DETACH).map_err(|e| Error::Nix {
            msg: "umount2",
            src: e,
        })?;

        // delete the old root mount point
        fs::remove_dir("old_root/")?;

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
