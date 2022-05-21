use log::{debug, error, info, trace};

use crate::clone::{clone3, CloneArgs, CloneFlags};
use crate::{Error, Result};

use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};

use nix::fcntl::{FcntlArg, FdFlag};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::unistd::{close, dup2, getgid, getuid, pivot_root, sethostname, Gid, Pid, Uid};

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
    hostname: Option<String>,
    domain_name: Option<String>,

    mounts: HashMap<PathBuf, PathBuf>,
    fds: HashSet<RawFd>,

    remount_proc: bool,
}

impl VoidBuilder {
    pub fn new() -> VoidBuilder {
        VoidBuilder {
            hostname: None,
            domain_name: None,
            mounts: HashMap::new(),
            fds: HashSet::new(),
            remount_proc: false,
        }
    }

    pub fn set_hostname<T: Into<String>>(&mut self, hostname: T) -> &mut Self {
        self.hostname = Some(hostname.into());
        self
    }

    pub fn set_domain_name<T: Into<String>>(&mut self, domain_name: T) -> &mut Self {
        self.domain_name = Some(domain_name.into());
        self
    }

    pub fn mount<T1: AsRef<Path>, T2: AsRef<Path>>(&mut self, src: T1, dst: T2) -> &mut Self {
        self.mounts.insert(src.as_ref().into(), dst.as_ref().into());
        self
    }

    pub fn keep_fd(&mut self, fd: &impl AsRawFd) -> &mut Self {
        self.fds.insert(fd.as_raw_fd());
        self
    }

    pub fn remount_proc(&mut self) -> &mut Self {
        self.remount_proc = true;
        self
    }

    pub fn spawn(&mut self, child_fn: impl FnOnce() -> i32) -> Result<VoidHandle> {
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

        let parent_uid = getuid();
        let parent_gid = getgid();

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
                debug!("voiding user namespace...");
                self.void_user_namespace(parent_uid, parent_gid)?; // first to regain full capabilities

                debug!("voiding mount namespace...");
                self.void_mount_namespace()?;
                debug!("voiding file descriptors..."); // occur after mount to unmount /dev/null
                self.void_file_descriptors()?;

                debug!("voiding ipc namespace...");
                self.void_ipc_namespace()?;
                debug!("voiding uts namespace...");
                self.void_uts_namespace()?;
                debug!("voiding network namespace...");
                self.void_network_namespace()?;
                debug!("voiding pid namespace...");
                self.void_pid_namespace()?;
                debug!("voiding cgroup namespace...");
                self.void_cgroup_namespace()?;

                Ok::<(), Error>(())
            };

            if let Err(e) = result {
                error!("error preparing void: {}", e);
                std::process::exit(-1)
            } else {
                info!("successfully prepared void");
                std::process::exit(child_fn())
            }
        }

        debug!("cloned child: {}", child);
        Ok(VoidHandle { pid: child })
    }

    /**
     * Voiding an ipc namespace requires no work. A newly created ipc namespace
     * contains nothing, and there is no sharing of ipc objects between
     * namespaces.
     */
    fn void_ipc_namespace(&self) -> Result<()> {
        Ok(())
    }

    /**
     * Voiding a uts namespace requires setting the host and domain names to
     * something specific. A newly created uts namespace has copies of the
     * parent values for each of these.
     */
    fn void_uts_namespace(&self) -> Result<()> {
        sethostname(self.hostname.as_deref().unwrap_or("void")).map_err(|e| Error::Nix {
            msg: "sethostname",
            src: e,
        })?;

        setdomainname(self.domain_name.as_deref().unwrap_or("(none)")).map_err(|e| Error::Nix {
            msg: "setdomainname",
            src: e,
        })?;

        Ok(())
    }

    /**
     * Voiding a network namespace requires no work. A newly created network
     * namespace contains only a loopback adapter, so is already a void.
     */
    fn void_network_namespace(&self) -> Result<()> {
        Ok(())
    }

    /**
     * Voiding a pid namespace requires no work. Creating a pid namespace means
     * the first process created is PID 1, which clone does immediately.
     */
    fn void_pid_namespace(&self) -> Result<()> {
        Ok(())
    }

    /**
     * Voiding a mount namespace replaces the current root with a new `tmpfs`.
     * This requires pivoting the old root and setting it to private before
     * unmounting it from the mount namespace. Filling the mount namespace with
     * bind mounts must also be done here, as the mounts are completely
     * unavailable after unmounting the old root.
     */
    fn void_mount_namespace(&self) -> Result<()> {
        trace!("changing the propagation type of the old root to private");
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

        trace!("creating tmpdir for new root");
        let tmp_base = {
            let env_dir = env::temp_dir();
            if env_dir.exists() {
                env_dir
            } else {
                debug!("env_dir does not exist, assuming `/` as the base");
                "/".into()
            }
        };

        // consume so it does not attempt to delete a folder which no longer exists
        let new_root = tempfile::tempdir_in(tmp_base)?.into_path();

        trace!("mounting a new root tmpfs at `{:?}`", &new_root);
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

        let put_old = new_root.join("old_root/");
        debug!("new_root: {:?}; put_old: {:?}", &new_root, &put_old);
        fs::create_dir_all(&put_old)?;

        trace!("pivoting old root into a subdirectory of new root");
        pivot_root(&new_root, &put_old).map_err(|e| Error::Nix {
            msg: "pivot_root",
            src: e,
        })?;

        let new_root = PathBuf::from("/");
        let old_root = PathBuf::from("/old_root/");

        trace!("changing root directory to new root");
        std::env::set_current_dir(&new_root)?;

        trace!("creating bind mounts before unmounting");

        let standard_dev_null = if self.mounts.contains_key(&PathBuf::from("/dev/null")) {
            None
        } else {
            Some((PathBuf::from("/dev/null"), PathBuf::from("/dev/null")))
        };

        for (src, dst) in self
            .mounts
            .iter()
            .chain(standard_dev_null.as_ref().map(|(x, y)| (x, y)))
        {
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

            // rbind mount
            mount(
                Some(&src),
                &dst,
                Option::<&str>::None,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                Option::<&str>::None,
            )
            .map_err(|e| Error::Nix {
                msg: "mount",
                src: e,
            })?;
        }

        // remount proc
        if self.remount_proc {
            debug!("remounting /proc`");

            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::empty(),
                Option::<&str>::None,
            )
            .map_err(|e| Error::Nix {
                msg: "mount",
                src: e,
            })?;
        }

        // unmount the old root
        umount2(&old_root, MntFlags::MNT_DETACH).map_err(|e| Error::Nix {
            msg: "umount2",
            src: e,
        })?;

        // delete the old root mount point
        fs::remove_dir(&old_root)?;

        Ok(())
    }

    /**
     * Voiding the user namespace requires writing to two mapping files, and disabling
     * setgid(2). The contents of the mapping files map back to the parent_uid and
     * parent_gid, which must be passed in as they are lost when the new namespace is
     * created.
     */
    fn void_user_namespace(&self, parent_uid: Uid, parent_gid: Gid) -> Result<()> {
        debug!("mapping root uid to {} in the parent", parent_uid);
        let mut uid_map = fs::OpenOptions::new()
            .read(false)
            .write(true)
            .open("/proc/self/uid_map")?;

        uid_map.write_all(format!("0 {} 1\n", parent_uid).as_ref())?;
        close(uid_map.into_raw_fd()).map_err(|e| Error::Nix {
            msg: "close",
            src: e,
        })?;

        debug!("writing deny to setgroups");
        let mut setgroups = fs::OpenOptions::new()
            .read(false)
            .write(true)
            .open("/proc/self/setgroups")?;

        setgroups.write_all("deny\n".as_bytes())?;
        close(setgroups.into_raw_fd()).map_err(|e| Error::Nix {
            msg: "close",
            src: e,
        })?;

        debug!("mapping root gid to {} in the parent", parent_gid);
        let mut gid_map = fs::OpenOptions::new()
            .read(false)
            .write(true)
            .open("/proc/self/gid_map")?;

        gid_map.write_all(format!("0 {} 1\n", parent_gid).as_ref())?;
        close(gid_map.into_raw_fd()).map_err(|e| Error::Nix {
            msg: "close",
            src: e,
        })?;

        Ok(())
    }

    /**
     * Voiding cgroups involves placing the process into a leaf before creating a
     * cgroup namespace. This ensures the view of the process does not exceed itself.
     */
    fn void_cgroup_namespace(&self) -> Result<()> {
        // TODO: void cgroup namespace
        Ok(())
    }

    /**
     * Voiding file descriptors closes all but specified file descriptors, and ensures
     * the remaining ones are not close-on-exec.
     */
    fn void_file_descriptors(&self) -> Result<()> {
        let mut closer = CloseFdsBuilder::new();

        let keep: Box<[RawFd]> = self.fds.iter().copied().collect();
        closer.keep_fds(&keep);

        unsafe {
            closer.closefrom(3);
        }

        // overwrite stdin/stdout/stderr without closing
        {
            let mut nullfd: Option<File> = None;
            for stdfd in &[0, 1, 2] {
                if !keep.contains(stdfd) {
                    trace!("voiding stdfd {}", stdfd);

                    let fd = nullfd
                        .take()
                        .map(Ok)
                        .unwrap_or_else(|| File::open("/dev/null"))?;

                    dup2(fd.as_raw_fd(), *stdfd).map_err(|e| Error::Nix {
                        msg: "dup2",
                        src: e,
                    })?;

                    nullfd = Some(fd);
                }
            }
        }

        if !self.mounts.contains_key(&PathBuf::from("/dev/null")) {
            debug!("unmount /dev/null after voiding file descriptors");

            umount2("/dev/null", MntFlags::MNT_DETACH).map_err(|e| Error::Nix {
                msg: "umount2",
                src: e,
            })?;
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
}

pub fn setdomainname<S: AsRef<std::ffi::OsStr>>(name: S) -> nix::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let ptr = name.as_ref().as_bytes().as_ptr() as *const libc::c_char;
    let len = name.as_ref().len() as libc::size_t;

    let res = unsafe { libc::setdomainname(ptr, len) };
    nix::Error::result(res).map(drop)
}
