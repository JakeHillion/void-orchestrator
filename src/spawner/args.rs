use super::{Spawner, TriggerData};
use crate::specification::{Arg, FileSocket, Pipe};
use crate::{Error, Result};

use std::ffi::CString;
use std::fs::File;
use std::net::TcpListener;
use std::os::unix::io::IntoRawFd;

/**
 * perform initial processing with ambient authority
 * for things like network sockets.
 */
pub struct PreparedArgs(Vec<PreparedArg>);

impl PreparedArgs {
    pub fn prepare_ambient(args: &[Arg]) -> Result<Self> {
        let mut v = Vec::with_capacity(args.len());

        for arg in args {
            v.push(PreparedArg::prepare_ambient(arg)?);
        }

        Ok(PreparedArgs(v))
    }

    pub(super) fn prepare_void_mut(
        self,
        spawner: &mut Spawner,
        entrypoint: &str,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        let mut v = Vec::new();

        for arg in self.0 {
            v.extend(arg.prepare_void_mut(spawner, entrypoint, trigger)?)
        }

        Ok(v)
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
    Pipe(Pipe),

    /// File socket
    FileSocket(FileSocket),

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
    fn prepare_ambient(arg: &Arg) -> Result<Self> {
        Ok(match arg {
            Arg::File(path) => PreparedArg::File(File::open(path)?),

            Arg::TcpListener { addr } => PreparedArg::TcpListener {
                socket: TcpListener::bind(addr)?,
            },

            Arg::BinaryName => PreparedArg::BinaryName,
            Arg::Entrypoint => PreparedArg::Entrypoint,
            Arg::Pipe(p) => PreparedArg::Pipe(p.clone()),
            Arg::FileSocket(s) => PreparedArg::FileSocket(s.clone()),
            Arg::Trigger => PreparedArg::Trigger,
            Arg::Trailing => PreparedArg::Trailing,
        })
    }

    /**
     * Complete argument preparation in the void
     */
    fn prepare_void_mut(
        self,
        spawner: &mut Spawner,
        entrypoint: &str,
        trigger: &mut TriggerData,
    ) -> Result<Vec<CString>> {
        match self {
            PreparedArg::Pipe(p) => match p {
                Pipe::Rx(s) => {
                    let pipe = spawner.pipes.get_mut(&s).unwrap().take_read()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
                Pipe::Tx(s) => {
                    let pipe = spawner.pipes.get_mut(&s).unwrap().take_write()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
            },

            PreparedArg::FileSocket(s) => match s {
                FileSocket::Rx(s) => {
                    let pipe = spawner.sockets.get_mut(&s).unwrap().take_read()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
                FileSocket::Tx(s) => {
                    let pipe = spawner.sockets.get_mut(&s).unwrap().take_write()?;
                    Ok(vec![CString::new(pipe.into_raw_fd().to_string()).unwrap()])
                }
            },

            arg => arg.prepare_void(spawner, entrypoint, trigger),
        }
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
            PreparedArg::BinaryName => Ok(vec![CString::new(spawner.binary).unwrap()]),
            PreparedArg::Entrypoint => Ok(vec![CString::new(entrypoint).unwrap()]),

            PreparedArg::Pipe(p) => Err(Error::BadPipe(p.get_name().to_string())),
            PreparedArg::FileSocket(s) => Err(Error::BadFileSocket(s.get_name().to_string())),

            PreparedArg::File(f) => Ok(vec![CString::new(f.into_raw_fd().to_string()).unwrap()]),

            PreparedArg::Trigger => Ok(trigger.args()),

            PreparedArg::TcpListener { socket } => {
                Ok(vec![CString::new(socket.into_raw_fd().to_string()).unwrap()])
            }

            PreparedArg::Trailing => Ok(spawner
                .trailing
                .iter()
                .map(|s| CString::new(*s).unwrap())
                .collect()),
        }
    }
}
