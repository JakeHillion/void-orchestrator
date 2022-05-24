use std::fs::File;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};

use nix::poll::{poll, PollFd, PollFlags};
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::Error as NixError;

use lazy_static::lazy_static;

lazy_static! {
    static ref RUNNING: AtomicBool = AtomicBool::new(true);
}

pub(crate) fn handler(tls_handler_trigger: File, listener: TcpListener) -> i32 {
    println!("connection_listener entered");

    // SAFETY: only unsafe if you use the result
    unsafe { signal(Signal::SIGINT, SigHandler::Handler(handle_sigint)) }.unwrap();

    listener.set_nonblocking(true).unwrap();

    let mut to_poll = [PollFd::new(listener.as_raw_fd(), PollFlags::POLLIN)];
    while RUNNING.load(Ordering::Relaxed) {
        if let Err(e) = poll(&mut to_poll, 1000) {
            if e == NixError::EINTR {
                continue; // timed out
            }
            Err(e).unwrap()
        }

        let stream = match listener.accept() {
            Ok(s) => s,
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    Err(e).unwrap()
                } else {
                    continue;
                }
            }
        };

        println!("received a new connection");
        super::tls_handler(&tls_handler_trigger, stream.0);
    }

    exitcode::OK
}

extern "C" fn handle_sigint(signal: libc::c_int) {
    let signal = Signal::try_from(signal).unwrap();
    RUNNING.store(signal != Signal::SIGINT, Ordering::Relaxed);
}
