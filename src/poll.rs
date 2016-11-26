use libc;

use std::fs::File;
use std::io::{BufRead,BufReader};
use std::os::unix::io::AsRawFd;

/// Set up a file to use with `poll`.
pub fn setup_pollfd(fd: &File) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0
    }
}

/// Poll a set of filedescriptors perviously wrapped.
pub fn poll(fds: &mut [libc::pollfd]) -> bool {
    let poll_res = unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as u64, -1)
    };
    poll_res > 0
}

/// A wrapped `BufReader` only yielding complete lines, annotated with
/// an index.
pub struct FileBuffer(pub Vec<u8>, pub BufReader<File>, pub String);

/// Fill some buffers from a set of previously `poll`ed filedsecriptors.
pub fn get_lines(fds: &[libc::pollfd], buffers: &mut [FileBuffer])
        -> Vec<(String, String)> {
    let fd_len = fds.len();
    let mut res = Vec::with_capacity(fd_len);
    for (fd, &mut FileBuffer(ref mut buf, ref mut reader, ref name)) in
            fds.iter().zip(buffers) {
        if fd.fd != reader.get_ref().as_raw_fd() {
            panic!("error: mismatched FileBuffer. please file an issue.");
        }

        if fd.revents & libc::POLLIN != 0 &&
            reader.read_until(0xA, buf).is_ok() {
            if let Some(&c) = buf.last() {
                if c == 0xA { let _ = buf.pop(); }
                if let Ok(s) = String::from_utf8(buf.clone()) {
                    res.push((name.clone(), s));
                }
                buf.clear();
            }
        }
    }
    res
}
