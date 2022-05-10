use super::{Spawner, TriggerData};
use crate::specification::{Arg, FileSocket, Pipe};
use crate::void::VoidBuilder;
use crate::{Error, Result};

use std::ffi::CString;
use std::fs::File;
use std::net::TcpListener;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::IntoRawFd;

pub struct PreparedArgs(Vec<PreparedArg>);

impl PreparedArgs {
    /**
     * perform initial processing with ambient authority
     * for things like network sockets. update the builder
     * with newly passed fds. the mutable call is allowed
     * to use up things such as pipes and sockets.
     */
    pub fn prepare_ambient_mut(
        spawner: &mut Spawner,
        builder: &mut VoidBuilder,
        args: &[Arg],
    ) -> Result<Self> {
        let mut v = Vec::with_capacity(args.len());

        for arg in args {
            v.push(PreparedArg::prepare_ambient_mut(spawner, builder, arg)?);
        }

        Ok(PreparedArgs(v))
    }

    /**
     * perform initial processing with ambient authority
     * for things like network sockets. update the builder
     * with newly passed fds.
     */
    pub fn prepare_ambient(
        spawner: &Spawner,
        builder: &mut VoidBuilder,
        args: &[Arg],
    ) -> Result<Self> {
        let mut v = Vec::with_capacity(args.len());

        for arg in args {
            v.push(PreparedArg::prepare_ambient(spawner, builder, arg)?);
        }

        Ok(PreparedArgs(v))
    }

    pub(super) fn prepare_void(
        self,
        spawner: &Spawner,
        entrypoint: &str,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        let mut v = Vec::new();

        for arg in self.0 {
            v.extend(arg.prepare_void(spawner, entrypoint, trigger)?)
        }

        Ok(v)
    }
}
enum PreparedArg {
    /// The binary name, or argv[0], of the original program start
    BinaryName,

    /// The name of this entrypoint
    Entrypoint,

    /// A file descriptor for a file on the filesystem in the launching namespace
    File(File),

    /// A chosen end of a named pipe
    Pipe(File),

    /// File socket
    FileSocket(File),

    /// A value specified by the trigger
    /// NOTE: Only valid if the trigger is of type Pipe(...) or FileSocket(...)
    Trigger,

    /// A TCP Listener
    TcpListener { socket: TcpListener },

    /// The rest of argv[1..], 0 or more arguments
    Trailing,
}

impl PreparedArg {
    /**
     * Process the parts of the argument which must be processed
     * with ambient authority
     *
     * Leave the remainder untouched so they can be processed in parallel
     * (in the child process) and to reduce authority
     */
    fn prepare_ambient_mut(
        spawner: &mut Spawner,
        builder: &mut VoidBuilder,
        arg: &Arg,
    ) -> Result<Self> {
        Ok(match arg {
            Arg::Pipe(p) => {
                let pipe = match p {
                    Pipe::Rx(s) => spawner.pipes.get_mut(s).unwrap().take_read(),
                    Pipe::Tx(s) => spawner.pipes.get_mut(s).unwrap().take_write(),
                }?;

                builder.keep_fd(&pipe);
                PreparedArg::Pipe(pipe)
            }

            Arg::FileSocket(FileSocket::Rx(s)) => {
                let socket = spawner.sockets.get_mut(s).unwrap().take_read()?;

                builder.keep_fd(&socket);
                PreparedArg::FileSocket(socket)
            }

            arg => Self::prepare_ambient(spawner, builder, arg)?,
        })
    }

    fn prepare_ambient(spawner: &Spawner, builder: &mut VoidBuilder, arg: &Arg) -> Result<Self> {
        Ok(match arg {
            Arg::Pipe(p) => return Err(Error::BadPipe(p.get_name().to_string())),
            Arg::FileSocket(FileSocket::Rx(s)) => return Err(Error::BadFileSocket(s.to_string())),

            Arg::FileSocket(FileSocket::Tx(s)) => {
                let socket = spawner.sockets.get(s).unwrap().write()?;

                builder.keep_fd(&socket);
                PreparedArg::FileSocket(socket)
            }

            Arg::File(path) => {
                let fd = File::open(path)?;
                builder.keep_fd(&fd);

                PreparedArg::File(fd)
            }

            Arg::TcpListener { addr } => {
                let socket = TcpListener::bind(addr)?;
                builder.keep_fd(&socket);

                PreparedArg::TcpListener { socket }
            }

            Arg::BinaryName => PreparedArg::BinaryName,
            Arg::Entrypoint => PreparedArg::Entrypoint,
            Arg::Trigger => PreparedArg::Trigger,
            Arg::Trailing => PreparedArg::Trailing,
        })
    }

    /**
     * Complete argument preparation in the void
     */
    fn prepare_void(
        self,
        spawner: &Spawner,
        entrypoint: &str,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        match self {
            PreparedArg::BinaryName => {
                Ok(vec![
                    CString::new(spawner.binary.as_os_str().as_bytes()).unwrap()
                ])
            }
            PreparedArg::Entrypoint => Ok(vec![CString::new(entrypoint).unwrap()]),

            PreparedArg::Pipe(p) => Ok(vec![CString::new(p.into_raw_fd().to_string()).unwrap()]),
            PreparedArg::FileSocket(s) => {
                Ok(vec![CString::new(s.into_raw_fd().to_string()).unwrap()])
            }

            PreparedArg::File(f) => Ok(vec![CString::new(f.into_raw_fd().to_string()).unwrap()]),

            PreparedArg::Trigger => Ok(trigger.args()),

            PreparedArg::TcpListener { socket } => {
                Ok(vec![CString::new(socket.into_raw_fd().to_string()).unwrap()])
            }

            PreparedArg::Trailing => Ok(spawner
                .binary_args
                .iter()
                .map(|s| CString::new(*s).unwrap())
                .collect()),
        }
    }
}
