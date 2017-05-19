use libc;

use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

pub fn open_fifo(path: &Path) -> Option<File> {
    // we open the file in read-write mode to prevent our poll()
    // hack from sending us `POLLHUP`s when no process is at the
    // other end of the pipe, so it blocks either way.
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => {
            match f.metadata().map(|m| m.file_type().is_fifo()) {
                Ok(true) => Some(f),
                _ => None, // regular file
            }
        }
        _ => {
            let path_ptr = path.as_os_str().to_os_string().to_str().unwrap().as_ptr();
            let perms = libc::S_IRUSR | libc::S_IWUSR;
            let ret = unsafe { libc::mkfifo(path_ptr as *const i8, perms) };
            if ret != 0 { None } else { open_fifo(path) }
        }
    }
}
