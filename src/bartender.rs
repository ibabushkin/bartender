//! Config Parser and Interpreter module.
//!
//! Presents types and functions to read in, represent and interpret data
//! found in configuration files for the software.

// a rather hackish wrapper around `mkfifo` to make sure we only touch
// the right files
use crate::mkfifo::open_fifo;

// an equally hackish wrapper around `poll` for proper I/O on FIFOs
use crate::poll;
use crate::poll::{FileBuffer, Message};

use mustache::{compile_str, Error, Template};

// I/O stuff for the heavy lifting, path lookup and similar things
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::env::home_dir;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Error as IoError, stdout};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};
use std::sync::mpsc;
use std::thread;

// timer stuff
use time::{Duration, SteadyTime, Timespec, get_time};

// config parsing machinery
use toml;
use toml::value::{Table, Value};

/// Config data.
///
/// Holds a number of input sources as well as an output buffer.
#[derive(Debug)]
pub struct Config {
    /// Compiled format template.
    format: Template,
    /// All timer sources.
    timers: TimerSet,
    /// All FIFO sources.
    fifos: FifoSet,
    /// A mapping from index to input name.
    id_mapping: Vec<String>,
}

impl Config {
    /// Parse a config file and return a result.
    pub fn from_config_file(file: &Path) -> ConfigResult<Config> {
        // attempt to parse configuration file
        let mut cfg = parse_config_file(file)?;

        let mut id_mapping = Vec::new();

        let template = if let Some(Value::String(format)) = cfg.remove("format") {
            let mut s = format.replace("\n", "");
            s.push('\n'); // TODO: wääh
            match compile_str(s.as_str()) {
                Ok(t) => t,
                Err(e) => return Err(ConfigError::MustacheError(e)),
            }
        } else {
            return Err(ConfigError::MissingFormat);
        };

        // get the set of Timers
        let timers = if let Some(Value::Table(timers)) = cfg.remove("timers") {
            let mut ts = Vec::with_capacity(timers.len());

            for (id, (name, timer)) in timers.into_iter().enumerate() {
                id_mapping.push(name.clone());
                ts.push(Timer::from_config(name, id, timer)?);
            }

            ts
        } else {
            Vec::new()
        };

        // get the set of FIFOs
        let fifos = if let Some(Value::Table(fifos)) = cfg.remove("fifos") {
            let mut fs = Vec::with_capacity(fifos.len());
            let mut id = timers.len();

            for (name, fifo) in fifos {
                id_mapping.push(name.clone());
                fs.push(Fifo::from_config(name.clone(), id, fifo)?);
                id += 1;
            }

            fs
        } else {
            Vec::new()
        };

        // return the results
        Ok(Config {
               format: template,
               timers: TimerSet { timers },
               fifos: FifoSet { fifos },
               id_mapping,
           })
    }

    /// Run with the given configuration.
    ///
    /// Create an MPSC channel passed to each thread spawned, each
    /// representing one of the entries (which is either FIFO or timer).
    /// The messages get merged into the buffer and the modified contents
    /// get stored.
    pub fn run(self) {
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let Config {
            format,
            timers,
            fifos,
            id_mapping,
        } = self;
        let mut last_input_results = HashMap::new();

        thread::spawn(move || { timers.run(tx); });

        thread::spawn(move || { fifos.run(tx2); });

        for updates in rx.iter() {
            for (id, value) in updates {
                last_input_results.insert(&id_mapping[id], value);
            }

            if let Err(e) = format.render(&mut stdout(), &last_input_results) {
                eprintln!("mustache error: {}", e);
            }
        }
    }
}

/// An error that occured during setup.
#[derive(Debug)]
pub enum ConfigError {
    /// I/O error occured.
    IOError(IoError),
    /// The file could not be parsed.
    TomlError(toml::de::Error),
    /// The TOML tree does not consist of a toplevel table.
    TomlNotTable,
    /// No format is specified in file.
    MissingFormat,
    /// Mustache template could not be parsed.
    MustacheError(Error),
    /// Some value is missing.
    Missing(String, Option<&'static str>),
    /// A timer is malformatted.
    IllegalValues(String),
    /// No home directory was found.
    NoHome,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            ConfigError::IOError(ref io_error) => write!(f, "I/O error occured: {}", io_error),
            ConfigError::TomlError(ref p) => write!(f, "TOML parsing failed: {}", p),
            ConfigError::TomlNotTable => write!(f, "TOML not consisting of a toplevel table"),
            ConfigError::MissingFormat => write!(f, "no `format` list found"),
            ConfigError::MustacheError(ref err) => write!(f, "format could not be parsed: {}", err),
            ConfigError::Missing(ref name, Some(sub)) => {
                write!(f,
                       "entity `{}` is missing child `{}` \
                       (or it has the wrong type)",
                       name,
                       sub)
            }
            ConfigError::Missing(ref name, None) => {
                write!(f,
                       "entity `{}` is missing \
                       (or it has the wrong type)",
                       name)
            }
            ConfigError::IllegalValues(ref name) => {
                write!(f, "timer `{}` doesn't have a positive period", name)
            }
            ConfigError::NoHome => write!(f, "no home directory found"),
        }
    }
}

/// Result wrapper.
type ConfigResult<T> = Result<T, ConfigError>;

/// Parse a configuration file - helper.
fn parse_config_file(path: &Path) -> ConfigResult<Table> {
    File::open(path)
        .map_err(ConfigError::IOError)
        .and_then(|mut file| {
            let mut content = String::new();

            file.read_to_string(&mut content)
                .map_err(ConfigError::IOError)
                .and_then(|_| {
                    match content.parse::<Value>() {
                        Ok(Value::Table(value)) => Ok(value),
                        Ok(_) => Err(ConfigError::TomlNotTable),
                        Err(err) => Err(ConfigError::TomlError(err)),
                    }
                })
        })
}

/// Parse a path - helper.
fn parse_path(path: &str) -> ConfigResult<PathBuf> {
    if path.starts_with("~/") {
        if let Some(dir) = home_dir() {
            Ok(dir.join(PathBuf::from(&path[2..])))
        } else {
            Err(ConfigError::NoHome)
        }
    } else {
        Ok(PathBuf::from(path))
    }
}

/// A timer source.
#[derive(Debug, PartialEq, Eq)]
struct Timer {
    /// Time interval between invocations.
    period: Duration,
    /// The command as a path buffer.
    command: String,
    /// The output destination of the timer.
    id: usize,
}

impl Timer {
    /// Parse a Timer from a config structure.
    fn from_config(name: String, id: usize, config: Value) -> ConfigResult<Timer> {
        if let Value::Table(mut table) = config {
            let seconds = if let Some(&Value::Integer(s)) = table.get("seconds") {
                s
            } else {
                0
            };

            // TODO: clean this up to avoid tons of lines (similar stuff for other config parsing
            // code) .map_or would be useful
            let minutes = if let Some(&Value::Integer(m)) = table.get("minutes") {
                m
            } else {
                0
            };

            let hours = if let Some(&Value::Integer(h)) = table.get("hours") {
                h
            } else {
                0
            };

            let command = if let Some(Value::String(c)) = table.remove("command") {
                c
            } else {
                return Err(ConfigError::Missing(name, Some("command")));
            };

            let period = Duration::seconds(seconds) + Duration::minutes(minutes) +
                         Duration::hours(hours);

            if period > Duration::seconds(0) {
                Ok(Timer { period, command, id })
            } else {
                Err(ConfigError::IllegalValues(name))
            }
        } else {
            Err(ConfigError::Missing(name, None))
        }
    }

    /// Execute one iteration of the command.
    fn execute(&self, tx: &mpsc::Sender<Message>) {
        if let Ok(output) = Command::new("sh").args(&["-c", &self.command]).output() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                let _ = tx.send(vec![(self.id, s.replace('\n', ""))]);
            }

            match output.status.code() {
                Some(0) => (),
                Some(c) => eprintln!("process \"{}\" exited with code {}", self.command, c),
                None => eprintln!("process \"{}\" got killed by signal", self.command),
            }
        }
    }
}

/// A type used to order events coming from `Timer`s.
#[derive(Debug, PartialEq, Eq)]
struct Entry<'a> {
    time: SteadyTime,
    timer: &'a Timer,
}

impl<'a> PartialOrd for Entry<'a> {
    fn partial_cmp(&self, other: &Entry) -> Option<Ordering> {
        //if self.time == other.time {
        //    self.timer.partial_cmp(&other.index).map(|c| c.reverse())
        //} else {
        self.time.partial_cmp(&other.time).map(|c| c.reverse())
        //}
    }
}

impl<'a> Ord for Entry<'a> {
    fn cmp(&self, other: &Entry) -> Ordering {
        // entries with the lowest time should come up first:
        //if self.time == other.time {
        //    self.index.cmp(&other.index).reverse()
        //} else {
        self.time.cmp(&other.time).reverse()
        //}
    }
}

/// A Set of timers, that get fired by a special worker thread.
#[derive(Debug)]
struct TimerSet {
    /// The actual timers and some info to direct their output.
    timers: Vec<Timer>,
}

impl TimerSet {
    /// Get the number of timers.
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// Run a worker thread handling `Timer`s.
    pub fn run(&self, tx: mpsc::Sender<Message>) {
        let len = self.len();
        let start_time = SteadyTime::now();
        let mut heap = BinaryHeap::with_capacity(len);

        // TODO: Suggestion: Insert sets of events into the heap, allowing for
        // simultaneous running of multiple events scheduled for the same
        // second. This could reduce jitter and improve the timers' sync
        // property - since less regenerating of the template takes place.
        // However, this could also increase visible latency and memory usage.
        for timer in &self.timers {
            heap.push(Entry {
                time: start_time,
                timer,
            });
        }

        while let Some(Entry { time, timer }) = heap.pop() {
            let now = SteadyTime::now();
            let period = timer.period.num_seconds();
            let sys_now = get_time();

            // we're not late
            if time > now {
                let max_next = (sys_now + (time - now)).sec;
                let next = Timespec::new(max_next - (max_next % period as i64), 0);

                if next > sys_now {
                    match (next - sys_now).to_std() {
                        Ok(duration) => thread::sleep(duration),
                        Err(e) => eprintln!("error: sleep failed: {}", e),
                    }
                }

                heap.push(Entry {
                    time: time + timer.period,
                    timer,
                });
            } else {
                let max_next = sys_now.sec + period;
                let next = Timespec::new(max_next - (max_next % period as i64), 0);

                heap.push(Entry {
                    time: time + (next - sys_now),
                    timer,
                });
            }

            timer.execute(&tx);
        }
    }
}

/// A FIFO source.
#[derive(Debug)]
struct Fifo {
    /// Path to FIFO.
    path: PathBuf,
    /// The output destination of the FIFO.
    id: usize,
    /// Default value used.
    default: Option<String>,
}

impl Fifo {
    /// Parse a Fifo from a config structure.
    fn from_config(name: String, id: usize, config: Value) -> ConfigResult<Fifo> {
        if let Value::Table(mut table) = config {
            let path = if let Some(&Value::String(ref c)) = table.get("fifo_path") {
                parse_path(c)?
            } else {
                return Err(ConfigError::Missing(name, Some("fifo_path")));
            };

            let default = if let Some(Value::String(d)) = table.remove("default") {
                Some(d)
            } else {
                None
            };

            Ok(Fifo { path, id, default })
        } else {
            Err(ConfigError::Missing(name, None))
        }
    }
}

#[derive(Debug)]
struct FifoSet {
    /// The actual FIFOs and some info to direct their output.
    fifos: Vec<Fifo>,
}

impl FifoSet {
    /// Run a worker thread handling `FIFO`s.
    pub fn run(mut self, tx: mpsc::Sender<Message>) {
        let len = self.fifos.len();
        let mut fds = Vec::with_capacity(len);
        let mut buffers = Vec::with_capacity(len);

        for fifo in self.fifos.drain(..) {
            if let Some(f) = open_fifo(&fifo.path) {
                if let Some(default) = fifo.default {
                    let _ = tx.send(vec![(fifo.id, default)]);
                }

                fds.push(poll::setup_pollfd(&f));
                buffers.push(FileBuffer(BufReader::new(f), fifo.id));
            } else {
                eprintln!(
                    "either a non-FIFO file {:?} exits, or it can't be created",
                    fifo.path
                );
                exit(1);
            }
        }

        drop(self);

        while poll::poll(&mut fds) {
            let _ = tx.send(poll::get_lines(&fds, &mut buffers));
        }
    }
}
