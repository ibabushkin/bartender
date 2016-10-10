//! Configuration Parser and Interpreter module.
//!
//! Presents types and functions to read in, represent and interpret data
//! found in configuration files for the software.

// some hacks for proper blocking
use c_helper::*;

// machinery to parse config file
use config::error::ConfigError;
use config::reader::from_file;
use config::types::{Config,ScalarValue,Setting,Value};

// I/O stuff for the heavy lifting, path lookup and similar things
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::env::home_dir;
use std::fmt;
use std::fs::OpenOptions;
use std::io::BufReader;
use std::io::prelude::*;
use std::path::{Path,PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration as StdDuration;
use std::time::Instant;

/// Configuration data.
///
/// Holds a number of input sources as well as an output buffer.
#[derive(Debug)]
pub struct Configuration {
    /// output buffer
    buffer: Buffer,
    /// all timer sources
    timers: TimerSet,
    /// all FIFO sources
    fifos: Vec<(usize, Fifo)>,
}

impl Configuration {
    /// Parse a config file and return a result.
    pub fn from_config_file(file: &Path) -> ConfigResult<Configuration> {
        // attempt to parse configuration file
        let cfg = try!(parse_config_file(file));

        // variables used for temporary storage and buildup of values
        let mut format_string = Vec::new();
        let mut timers = Vec::new();
        let mut fifos = Vec::new();

        // parse format information from config file
        let format =
            if let Some(&Value::List(ref l)) = cfg.lookup("format") {
                l
            } else {
                return Err(ConfigurationError::MissingFormat);
            };

        // iterate over format entries and store them
        for entry in format {
            match *entry {
                Value::Svalue(ScalarValue::Str(ref s)) =>
                    format_string.push(s.clone()),
                Value::Group(ref s) => {
                    let name = try!(get_nested_child(s, "name"));
                    try!(lookup_format_entry(&cfg, &mut timers, &mut fifos,
                                             name, format_string.len()));
                    let d = get_nested_child(s, "default").unwrap_or("");
                    format_string.push(String::from(d));
                },
                _ => return Err(ConfigurationError::IllegalFormat),
            }
        }

        // return the results
        Ok(Configuration {
            buffer: Buffer { format: format_string },
            timers: TimerSet { timers: timers },
            fifos: fifos,
        })
    }

    /// Run with the given configuration.
    ///
    /// Create a MPSC channel passed to each thread spawned, each representing
    /// one of the entries (which is either FIFO or timer). The messages get
    /// merged into the buffer and the modified contents get stored.
    pub fn run(mut self) {
        let (tx, rx) = mpsc::channel();

        {
            let tx = tx.clone();
            let timers = self.timers;
            thread::spawn(move || {
                timers.run(tx);
            });
        }

        for (index, fifo) in self.fifos.drain(..) {
            let tx = tx.clone();
            thread::spawn(move || {
                fifo.run(index, tx);
            });
        }

        for (index, value) in rx.iter() {
            self.buffer.set(index, value);
            self.buffer.output();
        }
    }
}

/// An error that occured during setup.
pub enum ConfigurationError {
    /// The file could not be parsed.
    ParsingError(ConfigError),
    /// No format is specified in file.
    MissingFormat,
    /// The format is malformatted (what irony).
    IllegalFormat,
    /// A nested entry is missing a child.
    MissingChild(String, String),
    /// A `type` value of a nested entry has an illegal value.
    IllegalType(String),
    NoHome,
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            ConfigurationError::ParsingError(ref c) =>
                write!(f, "parsing error: {}", c),
            ConfigurationError::MissingFormat =>
                write!(f, "no `format` list found"),
            ConfigurationError::IllegalFormat =>
                write!(f, "`format` list contains illegal entry"),
            ConfigurationError::MissingChild(ref p, ref c) =>
                write!(f, "object {} misses child {}", p, c),
            ConfigurationError::IllegalType(ref t) =>
                write!(f, "{} is not a valid `type` value", t),
            ConfigurationError::NoHome =>
                write!(f, "no home directory found"),
        }
    }
}

/// Result wrapper.
pub type ConfigResult<T> = Result<T, ConfigurationError>;

/// Parse a configuration file - helper.
fn parse_config_file(file: &Path) -> ConfigResult<Config> {
    match from_file(file) {
        Ok(cfg) => Ok(cfg),
        Err(e) => Err(ConfigurationError::ParsingError(e)),
    }
}

/// Parse a path - helper.
fn parse_path(path: &str) -> ConfigResult<PathBuf> {
    if path.starts_with("~/") {
        if let Some(dir) = home_dir() {
            Ok(dir.join(PathBuf::from(&path[2..])))
        } else {
            Err(ConfigurationError::NoHome)
        }
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Get a child element from a nested entry - helper.
fn get_child<'a>(cfg: &'a Config, name: &str, child: &str)
    -> ConfigResult<&'a str> {
    if let Some(value) =
        cfg.lookup_str(format!("{}.{}", name, child).as_str()) {
        Ok(value)
    } else {
        Err(ConfigurationError::MissingChild(
                String::from(name),
                String::from(child)
        ))
    }
}

/// Get a child element from a nested entry in format specifier - helper.
fn get_nested_child<'a>(s: &'a HashMap<String, Setting>, name: &str)
    -> ConfigResult<&'a str> {
    if let Some(&Setting {
        value: Value::Svalue(ScalarValue::Str(ref val)),
        ..
    }) = s.get(name) {
        Ok(val)
    } else {
        Err(ConfigurationError::IllegalFormat)
    }
}

/// Look up a format entry by name - helper.
fn lookup_format_entry(cfg: &Config,
                       timers: &mut Vec<(usize, Timer)>,
                       fifos: &mut Vec<(usize, Fifo)>,
                       name: &str, index: usize)
    -> ConfigResult<()> {
    let t = try!(get_child(&cfg, &name, "type"));
    if t == "timer" {
        let path = try!(get_child(&cfg, &name, "command"));
        timers.push((index, Timer {
            duration: StdDuration::from_secs(cfg.lookup_integer64_or(
                format!("{}.seconds", name).as_str(), 1) as u64),
            sync: cfg.lookup_boolean_or(
                format!("{}.sync", name).as_str(), false),
            command: String::from(path),
        }));
        Ok(())
    } else if t == "fifo" {
        let path = try!(get_child(&cfg, &name, "fifo_path"));
        fifos.push((index, Fifo {
            path: try!(parse_path(path)),
        }));
        Ok(())
    } else {
        Err(ConfigurationError::IllegalType(String::from(name)))
    }
}

/// A timer source.
#[derive(Debug)]
pub struct Timer {
    /// Time interval between invocations.
    duration: StdDuration,
    /// Sync to full minute on first/second iteration.
    sync: bool, // TODO
    /// The command as a path buffer
    command: String,
}

impl Timer {
    /*/// Run a timer input handler.
    ///
    /// Spawned in a separate thread, return a message for each time the
    /// command gets executed between sleep periods.
    pub fn run(&self, index: usize, tx: mpsc::Sender<(usize, String)>) {
        if self.sync {
            let now = now();
            let first_wait = (Duration::seconds((60 - now.tm_sec - 1) as i64) +
                Duration::nanoseconds(((10^9) - now.tm_nsec - 1) as i64))
                .to_std().unwrap();
            self.execute(index, &tx);
            thread::sleep(first_wait);
        }
        loop {
            self.execute(index, &tx);
            thread::sleep(self.duration);
        }
    }*/

    /// Execute one iteration of the command.
    fn execute(&self, index: usize, tx: &mpsc::Sender<(usize, String)>) {
        if let Ok(output) = Command::new("sh")
            .args(&["-c", &self.command]).output() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                let _ = tx.send((index, s));
            }

            macro_rules! err {
                ($format:expr, $($arg:expr),*) => {{
                    use std::io::stderr;
                    let _ =
                        writeln!(&mut stderr(), $format, $($arg),*);
                }}
            }

            match output.status.code() {
                Some(0) => (),
                Some(c) =>
                    err!("process \"{}\" exited with code {}",
                         self.command, c),
                None =>
                    err!("process \"{}\" got killed by signal", self.command),
            }
        }
    }
}

#[derive(PartialEq, Eq)]
struct Entry(Instant, usize);

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Entry) -> Option<Ordering> {
        if self.0 == other.0 {
            self.1.partial_cmp(&other.1).map(|c| c.reverse())
        } else {
            self.0.partial_cmp(&other.0).map(|c| c.reverse())
        }
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Entry) -> Ordering {
        if self.0 == other.0 {
            self.1.cmp(&other.1).reverse()
        } else {
            self.0.cmp(&other.0).reverse()
        }
    }
}

/// A Set of timers, that get fired by a special worker thread.
#[derive(Debug)]
pub struct TimerSet {
    /// The actual timers and some info to direct their output.
    timers: Vec<(usize, Timer)>,
}

impl TimerSet {
    /// Run a worker thread.
    pub fn run(&self, tx: mpsc::Sender<(usize, String)>) {
        let len = self.timers.len();
        let start_time = Instant::now();
        let mut heap = BinaryHeap::with_capacity(len);

        for index in 0..len {
            heap.push(Entry(start_time, index));
        }

        while let Some(Entry(timestamp, index)) = heap.pop() {
            let now = Instant::now();
            if timestamp > now {
                thread::sleep(timestamp - now);
            }
            if let Some(&(target_index, ref timer)) = self.timers.get(index) {
                timer.execute(target_index, &tx);
                heap.push(Entry(timestamp + timer.duration, index));
            }
        }
    }
}

/// A FIFO source.
#[derive(Debug)]
pub struct Fifo {
    /// Path to FIFO.
    path: PathBuf,
}

impl Fifo {
    /// Run a FIFO input handler.
    ///
    /// Spawned in a separate thread, return a message with a given index
    /// for each line received.
    pub fn run(&self, index: usize, tx: mpsc::Sender<(usize, String)>) {
        if let Ok(f) =
            OpenOptions::new().read(true).write(true).open(&self.path) {
            // we open the file in read-write mode to prevent our poll()
            // hack from sending us `POLLHUP`s when no process is at the
            // other end of the pipe, so it blocks either way.
            let mut file = BufReader::new(f);
            let mut buf = Vec::new();
            let mut pollfd = setup_pollfd(file.get_ref());
            loop {
                wait_for_data(&mut pollfd);
                if file.read_until(0xA, &mut buf).is_ok() {
                    if let Some(&c) = buf.last() {
                        if c == 0xA { let _ = buf.pop(); }
                        if let Ok(s) = String::from_utf8(buf) {
                            let _ = tx.send((index, s));
                        }
                        buf = Vec::new();
                    }
                }
            }
        } else {
            panic!("file could not be opened");
        }
    }
}

pub struct FifoSet {
    /// The actual FIFOs and some info to direct their output.
    fifos: Vec<(usize, Fifo)>,
}

impl FifoSet {
    /// Run a worker thread.
    pub fn run(&self, tx: mpsc::Sender<(usize, String)>) {
        let len = self.fifos.len();
        let mut fds = Vec::with_capacity(len);
        let mut buffers = Vec::with_capacity(len);

        for &(index, ref fifo) in self.fifos.iter() {
            if let Ok(f) =
                OpenOptions::new().read(true).write(true).open(&fifo.path) {
                fds.push(setup_pollfd(&f));
                buffers.push(FileBuffer(Vec::new(), BufReader::new(f), index));
            } else {
                panic!("file could not be opened");
            }
        }
    }
}

/// An Output buffer.
#[derive(Debug)]
pub struct Buffer {
    /// Format as a vector of strings that can be adressed (and changed)
    format: Vec<String>,
}

impl Buffer {
    /// Set the value at a given index.
    fn set(&mut self, index: usize, value: String) {
        self.format[index] = value.replace('\n', "");
    }

    /// Format everything
    fn output(&self) {
        println!("{}", self.format.join(""));
    }
}
