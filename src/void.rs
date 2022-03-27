use crate::clone::{clone3, CloneArgs, CloneFlags};
use crate::Error;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use nix::mount::{mount, umount, MsFlags};
use nix::sys::signal::Signal;
use nix::unistd::{pivot_root, Pid};

pub struct VoidHandle {}

pub struct VoidBuilder {
    #[allow(dead_code)]
    mounts: HashMap<PathBuf, PathBuf>,
}

impl VoidBuilder {
    pub fn new() -> VoidBuilder {
        VoidBuilder {
            mounts: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn mount(&mut self, src: PathBuf, dst: PathBuf) -> &mut Self {
        self.mounts.insert(src, dst);
        self
    }

    pub fn spawn(&mut self, child_fn: impl FnOnce() -> i32) -> Result<VoidHandle, Error> {
        let mut args = CloneArgs::new(
            CloneFlags::CLONE_NEWCGROUP
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWNET
                | CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWUSER
                | CloneFlags::CLONE_NEWUTS,
        );
        args.exit_signal = Some(Signal::SIGCHLD);

        let child = clone3(args).map_err(|e| Error::Nix {
            msg: "clone3",
            src: e,
        })?;

        if child == Pid::from_raw(0) {
            self.newns_post().unwrap();

            std::process::exit(child_fn())
        }

        // Leak the child function's resources in the parent process.
        // This avoids closing files that have been "moved" into the child.
        // It is also an over-approximation, and may cause actual memory leaks.
        // As the spawning process is normally short lived, this shouldn't
        // be a problem.
        std::mem::forget(child_fn);

        Ok(VoidHandle {})
    }

    // per-namespace void creation
    fn newns_post(&self) -> Result<(), Error> {
        // consume the TempDir so it doesn't get deleted
        let new_root = tempfile::tempdir()?.into_path();

        mount(
            Option::<&str>::None,
            &new_root,
            Some("tmpfs"),
            MsFlags::empty(),
            Option::<&str>::None,
        )
        .map_err(|e| Error::Nix {
            msg: "mount",
            src: e,
        })?;

        // TODO: Mount mounts

        let old_root = new_root.join("old_root/");
        fs::create_dir(&old_root)?;

        pivot_root(&new_root, &old_root).map_err(|e| Error::Nix {
            msg: "pivot_root",
            src: e,
        })?;
        std::env::set_current_dir("/")?;

        umount("old_root/").map_err(|e| Error::Nix {
            msg: "umount",
            src: e,
        })?;

        Ok(())
    }
}
