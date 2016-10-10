use libc;

use std::fs::File;
use std::io::{BufRead,BufReader};
use std::os::unix::io::AsRawFd;

/// Do... unspeakable things.
///
/// Waits for a filedescriptor to yield a value because the default API doesn't
/// give us this option. Calls `poll(3)` on the file descriptor opened with an
/// infinite timeout and returns once data is available.
pub fn wait_for_data(fd: &mut libc::pollfd) {
    unsafe {
        if libc::poll(fd as *mut libc::pollfd, 1, -1) > 0 &&
            (*fd).revents & libc::POLLIN != 0 {
            return;
        }
        unreachable!();
    }
}

/// Do... more unspeakable things.
///
/// Setup `pollfd` structure to use with the above waiting routine.
pub fn setup_pollfd(fd: &File) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0
    }
}

pub fn poll(fds: &mut [libc::pollfd]) -> bool {
    let poll_res = unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as u64, -1)
    };
    poll_res > 0
}

pub struct FileBuffer(pub Vec<u8>, pub BufReader<File>, pub usize);

pub fn get_lines(fds: &[libc::pollfd], buffers: &mut [FileBuffer])
    -> Vec<(usize, String)> {
    let fd_len = fds.len();
    let mut res = Vec::with_capacity(fd_len);
    for (fd, &mut FileBuffer(ref mut buf, ref mut reader, index)) in
        fds.iter().zip(buffers) {
        if fd.fd != reader.get_ref().as_raw_fd() {
            panic!("mismatched FileBuffer");
        }
        if fd.revents & libc::POLLIN != 0 {
            if reader.read_until(0xA, buf).is_ok() {
                if let Some(&c) = buf.last() {
                    if c == 0xA { let _ = buf.pop(); }
                    if let Ok(s) = String::from_utf8(buf.clone()) {
                        res.push((index, s));
                    }
                    buf.clear();
                }
            }
        }
    }
    res
}
