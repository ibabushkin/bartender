//! Config Parser and Interpreter module.
//!
//! Presents types and functions to read in, represent and interpret data
//! found in configuration files for the software.

#[macro_export]
macro_rules! err {
    ($format:expr, $($arg:expr),*) => {{
        use std::io::stderr;
        let _ =
            writeln!(&mut stderr(), $format, $($arg),*);
    }}
}

// a rather hackish wrapper around `poll` for proper I/O on FIFOs
use poll;
use poll::FileBuffer;

use mustache::{compile_str, Template};

// I/O stuff for the heavy lifting, path lookup and similar things
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::env::home_dir;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Error as IoError, stdout};
use std::io::prelude::*;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path,PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use time::{Duration,SteadyTime,Timespec,get_time};

use toml;
use toml::Value;

/// A channel we send our messages through.
type Channel = mpsc::Sender<Vec<(String, String)>>;

/// Config data.
///
/// Holds a number of input sources as well as an output buffer.
#[derive(Debug)]
pub struct Config {
    /// compiled format template
    format_template: Template,
    /// current values passed to the template
    last_input_results: HashMap<String, String>,
    /// all timer sources
    timers: TimerSet,
    /// all FIFO sources
    fifos: FifoSet,
}

impl Config {
    /// Parse a config file and return a result.
    pub fn from_config_file(file: &Path) -> ConfigResult<Config> {
        // attempt to parse configuration file
        let cfg = try!(parse_config_file(file));

        let mut inputs = HashMap::new();

        let template =
            if let Some(&Value::String(ref format)) = cfg.get("format") {
                let mut s = format.replace("\n", "");
                s.push('\n'); // TODO: wääh
                compile_str(s.as_str())
            } else {
                return Err(ConfigError::MissingFormat);
            };

        // get the set of Timers
        let timers =
            if let Some(&Value::Table(ref timers)) = cfg.get("timers") {
                let mut ts = Vec::with_capacity(timers.len());

                for (name, timer) in timers {
                    inputs.insert(name.clone(), String::from(""));
                    ts.push(try!(Timer::from_config(name.clone(), &timer)));
                }

                ts
            } else {
                Vec::new()
            };

        // get the set of FIFOs
        let fifos =
            if let Some(&Value::Table(ref fifos)) = cfg.get("fifos") {
                let mut fs = Vec::with_capacity(fifos.len());

                for (name, fifo) in fifos {
                    let (default, fifo) =
                        try!(Fifo::from_config(name.clone(), &fifo));
                    inputs.insert(name.clone(), default);
                    fs.push(fifo);
                }

                fs
            } else {
                Vec::new()
            };

        // return the results
        Ok(Config {
            format_template: template,
            last_input_results: inputs,
            timers: TimerSet { timers: timers },
            fifos: FifoSet { fifos: fifos },
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

        {
            let fifos = self.fifos;
            thread::spawn(move || {
                fifos.run(tx);
            });
        }

        for updates in rx.iter() {
            for (name, value) in updates {
                self.last_input_results.insert(name, value);
            }
            self.format_template.render(&mut stdout(), &self.last_input_results).unwrap();
        }
    }
}

/// An error that occured during setup.
#[derive(Debug)]
pub enum ConfigError {
    /// I/O error occured.
    IOError(IoError),
    /// File contains something other than UTF-8.
    BadEncoding,
    /// The file could not be parsed.
    TomlError(Vec<toml::ParserError>),
    /// No format is specified in file.
    MissingFormat,
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
            ConfigError::IOError(ref io_error) =>
                write!(f, "I/O error occured: {}", io_error),
            ConfigError::BadEncoding =>
                write!(f, "file has to be UTF-8 encoded"),
            ConfigError::TomlError(ref c) =>
                write!(f, "parsing error: {:?}", c),
            ConfigError::MissingFormat =>
                write!(f, "no `format` list found"),
            ConfigError::Missing(ref name, Some(sub)) =>
                write!(f, "entity `{}` is missing child `{}` \
                       (or it has the wrong type)", name, sub),
            ConfigError::Missing(ref name, None) =>
                write!(f, "entity `{}` is missing \
                       (or it has the wrong type)", name),
            ConfigError::IllegalValues(ref name) =>
                write!(f, "timer `{}` doesn't have a positive period", name),
            ConfigError::NoHome =>
                write!(f, "no home directory found"),
        }
    }
}

/// Result wrapper.
type ConfigResult<T> = Result<T, ConfigError>;

/// Parse a configuration file - helper.
fn parse_config_file(path: &Path) -> ConfigResult<toml::Table> {
    match File::open(path) {
        Ok(mut file) => {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_ok() {
                let mut parser = toml::Parser::new(&content);
                if let Some(value) = parser.parse() {
                    Ok(value)
                } else {
                    Err(ConfigError::TomlError(parser.errors))
                }
            } else {
                Err(ConfigError::BadEncoding)
            }
        },
        Err(io_error) => {
            Err(ConfigError::IOError(io_error))
        }
    }
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
    /// The command as a path buffer
    command: String,
    /// Where to write the output to
    name: String
}

impl Timer {
    fn from_config(name: String, config: &Value) -> ConfigResult<Timer> {
        if let Value::Table(ref table) = *config {
            let seconds =
                if let Some(&Value::Integer(s)) = table.get("seconds") {
                    s
                } else { 0 };

            let minutes =
                if let Some(&Value::Integer(m)) = table.get("minutes") {
                    m
                } else { 0 };

            let hours =
                if let Some(&Value::Integer(h)) = table.get("hours") {
                    h
                } else { 0 };

            let command =
                if let Some(&Value::String(ref c)) = table.get("command") {
                    c.clone()
                } else {
                    return Err(ConfigError::Missing(name, Some("command")));
                };

            let period = Duration::seconds(seconds) +
                Duration::minutes(minutes) + Duration::hours(hours);

            if period > Duration::seconds(0) {
                Ok(Timer {
                    period: period,
                    command: command,
                    name: name,
                })
            } else {
                Err(ConfigError::IllegalValues(name))
            }
        } else {
            Err(ConfigError::Missing(name, None))
        }
    }

    /// Execute one iteration of the command.
    fn execute(&self, tx: &Channel) {
        if let Ok(output) = Command::new("sh")
            .args(&["-c", &self.command]).output() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                let _ = tx.send(vec![(self.name.clone(), s.replace('\n', ""))]);
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
    /// Run a worker thread handling `Timer`s.
    pub fn run(&self, tx: Channel) {
        let len = self.timers.len();
        let start_time = SteadyTime::now();
        let mut heap = BinaryHeap::with_capacity(len);

        for timer in &self.timers {
            heap.push(Entry{ time: start_time, timer: timer });
        }

        while let Some(Entry{ time, timer }) = heap.pop() {
            let now = SteadyTime::now();

            // we're not late
            if time > now {
                let period = timer.period.num_seconds();
                let sys_now = get_time();
                let max_next = (sys_now + (time - now)).sec;
                let next =
                    Timespec::new(max_next - (max_next % period as i64), 0);

                let sleep_for = next - sys_now;
                if sleep_for > Duration::seconds(0) {
                    thread::sleep(sleep_for.to_std().unwrap());
                }
            }

            timer.execute(&tx);
            heap.push(Entry{ time: time + timer.period, timer: timer });
        }
    }
}

/// A FIFO source.
#[derive(Debug)]
struct Fifo {
    /// Path to FIFO.
    path: PathBuf,
    /// Where to write the output to
    name: String
}

impl Fifo {
    fn from_config(name: String, config: &Value)
        -> ConfigResult<(String, Fifo)> {
        if let Value::Table(ref table) = *config {
            let path =
                if let Some(&Value::String(ref c)) = table.get("fifo_path") {
                    try!(parse_path(c))
                } else {
                    return Err(
                        ConfigError::Missing(name, Some("fifo_path")));
                };

            let default =
                if let Some(&Value::String(ref d)) = table.get("default") {
                    d.clone()
                } else {
                    String::new()
                };

            Ok((default, Fifo { path: path, name: name }))
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
    pub fn run(&self, tx: Channel) {
        let len = self.fifos.len();
        let mut fds = Vec::with_capacity(len);
        let mut buffers = Vec::with_capacity(len);

        for fifo in &self.fifos {
            if let Ok(f) =
                OpenOptions::new().read(true).write(true).open(&fifo.path) {
                // we open the file in read-write mode to prevent our poll()
                // hack from sending us `POLLHUP`s when no process is at the
                // other end of the pipe, so it blocks either way.
                match f.metadata().map(|m| m.file_type().is_fifo()) {
                    Ok(true) => {
                        fds.push(poll::setup_pollfd(&f));
                        buffers.push(FileBuffer(Vec::new(),
                            BufReader::new(f), fifo.name.clone()));
                    },
                    _ => {
                        err!("{:?} is not a FIFO", fifo.path);
                    },
                }
            } else {
                err!("file {:?} could not be opened", fifo.path);
            }
        }

        while poll::poll(&mut fds) {
            let _ = tx.send(poll::get_lines(&fds, &mut buffers));
        }
    }
}
