use std::fs::File;
use std::io::Write;

fn main() {
    use std::os::unix::io::FromRawFd;

    let mut args = std::env::args();

    let _bin = args.next();

    match args.next() {
        Some(s) => match s.as_str() {
            "pipe_sender" => {
                let fd: i32 = args.next().unwrap().parse().unwrap();
                pipe_sender(unsafe { File::from_raw_fd(fd) })
            }
            "pipe_receiver" => {
                let pipe_data = args.next().unwrap();
                pipe_receiver(pipe_data.as_str())
            }
            _ => unimplemented!(),
        },
        None => unimplemented!(),
    }
}

fn pipe_sender(mut tx_pipe: File) {
    println!("hello from pipe_sender!");

    let data = b"some data";
    let bytes_written = tx_pipe.write(&data[..]).unwrap();
    assert!(bytes_written == data.len());

    let data = b"some more data";
    let bytes_written = tx_pipe.write(&data[..]).unwrap();
    assert!(bytes_written == data.len());
}

fn pipe_receiver(rx_data: &str) {
    println!("hello from pid: {}", std::process::id());
    println!("received data: {}", rx_data);
}
