//! Configuration Parser module.
//!
//! Presents types and functions to read in, represent and interpret data
//! found in configuration files for the software.

// some hacks for proper blocking
use c_helper::{wait_for_data,setup_pollfd};

// machinery to parse config file
use config::error::ConfigError;
use config::reader::from_file;
use config::types::{Config,ScalarValue,Setting,Value};

use std::env::home_dir;
use std::fmt;
// I/O stuff for the heavy lifting
use std::fs::OpenOptions;
use std::io::BufReader;
use std::io::prelude::*;
use std::path::{Path,PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Configuration data.
///
/// Holds a number of input sources as well as an output buffer.
#[derive(Debug)]
pub struct Configuration {
    /// output buffer
    buffer: Buffer,
    /// all timer sources
    timers: Vec<(usize, Timer)>,
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
        let mut entries = Vec::new();

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
                Value::Group(ref s) =>
                    if let Some(&Setting {
                            value: Value::Svalue(ScalarValue::Str(ref name)),
                            ..
                        }) = s.get("name") {
                        entries.push((name.clone(), format_string.len()));
                        format_string.push(String::new());
                    } else {
                        return Err(ConfigurationError::IllegalFormat);
                    },
                _ => return Err(ConfigurationError::IllegalFormat),
            }
        }

        // more buildup variables
        let mut timers = Vec::new();
        let mut fifos = Vec::new();

        // build up the sources
        for (ref name, index) in entries {
            let t = try!(get_child(&cfg, &name, "type"));
            if t == "timer" {
                let path = try!(get_child(&cfg, &name, "command_path"));
                timers.push((index, Timer {
                    seconds: get_seconds(&cfg, name),
                    command: String::from(path),
                }));
            } else if t == "fifo" {
                let path = try!(get_child(&cfg, &name, "fifo_path"));
                fifos.push((index, Fifo {
                    path: try!(parse_path(path)),
                }));
            } else {
                return Err(
                    ConfigurationError::IllegalType(name.clone())
                );
            }
        }

        // return the results
        Ok(Configuration {
            buffer: Buffer { format: format_string },
            timers: timers,
            fifos: fifos,
        })
    }

    /// Run with the given configuration.
    ///
    /// Create a MPSC channel passed to each thread spawned, each representing
    /// one of the entries (which is either FIFO or timer). The messages get
    /// merged into the buffer and the modified contents get stored.
    pub fn run(&mut self) {
        let (tx, rx) = mpsc::channel();

        for (index, timer) in self.timers.drain(..) {
            let tx = tx.clone();
            thread::spawn(move || {
                timer.run(index, tx);
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

/// Get a `seconds` value from a nested entry - helper.
fn get_seconds(cfg: &Config, name: &str) -> u32 {
    cfg.lookup_integer32_or(format!("{}.seconds", name).as_str(), 1) as u32
}

/// A timer source.
#[derive(Debug)]
pub struct Timer {
    /// The number of seconds between each invocation of the command.
    seconds: u32,
    /// The command as a path buffer
    command: String,
}

impl Timer {
    /// Run a timer input handler.
    ///
    /// Spawned in a separate thread, return a message for each time the
    /// command gets executed between sleep periods.
    pub fn run(&self, index: usize, tx: mpsc::Sender<(usize, String)>) {
        let duration = Duration::new(self.seconds as u64, 0);
        loop {
            if let Ok(output) = Command::new("sh")
                .args(&["-c", &self.command]).output() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    let _ = tx.send((index, s));
                }
            }
            thread::sleep(duration);
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
