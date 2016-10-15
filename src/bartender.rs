//! Configuration Parser and Interpreter module.
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

// machinery to parse config file
use mustache::{compile_str, Template};
use config::error::ConfigError;
use config::reader::from_str;
use config::types::{Config,Value};

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

/// A channel we send our messages through.
type Channel = mpsc::Sender<Vec<(String, String)>>;

/// Configuration data.
///
/// Holds a number of input sources as well as an output buffer.
#[derive(Debug)]
pub struct Configuration {
    format_template: Template,
    last_input_results: HashMap<String, String>,
    /// all timer sources
    timers: TimerSet,
    /// all FIFO sources
    fifos: FifoSet,
}

impl Configuration {
    /// Parse a config file and return a result.
    pub fn from_config_file(file: &Path) -> ConfigResult<Configuration> {
        // attempt to parse configuration file
        let (template, cfg) = try!(parse_config_file(file));

        // variables used for temporary storage and buildup of values
        //let mut format_string = Vec::new();
        let mut timers = Vec::new();
        let mut fifos = Vec::new();

        let inputs = match cfg.lookup("workaround").unwrap() {
            &Value::Group(ref i) => i,
            _ => {
                panic!();
            }
        };
        for name in inputs.keys() {
            try!(lookup_format_entry(&cfg, &mut timers, &mut fifos, name.as_str()));
        }

        // return the results
        Ok(Configuration {
            format_template: template,
            last_input_results: HashMap::new(),
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
pub enum ConfigurationError {
    /// The file is inaccessible.
    Inaccessible(IoError),
    /// No snip line found.
    NoSnip,
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
    /// A `seconds` value was non-positive.
    IllegalValue(String),
    /// No home directory was found.
    NoHome,
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            ConfigurationError::Inaccessible(ref io_error) =>
                write!(f, "file is inaccessible: {}", io_error),
            ConfigurationError::NoSnip =>
                write!(f, "no {} found", SNIP_BY),
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
            ConfigurationError::IllegalValue(ref t) =>
                write!(f, "{} should specify a positive timespan.", t),
            ConfigurationError::NoHome =>
                write!(f, "no home directory found"),
        }
    }
}

/// Result wrapper.
type ConfigResult<T> = Result<T, ConfigurationError>;

const SNIP_BY: &'static str = "================ BARTENDER-SNIP ================";

/// Parse a configuration file - helper.
fn parse_config_file(path: &Path) -> ConfigResult<(Template, Config)> {
    match File::open(path) {
        Ok(mut file) => {
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            let mut split = content.splitn(2, SNIP_BY);
            let format_template_str = split.next().unwrap();
            let input_cfg_str = match split.next() {
                Some(c) => c,
                None => {
                    return Err(ConfigurationError::NoSnip);
                }
            };
            let mut joined_template_string = format_template_str.replace("\n", "");
            joined_template_string.push('\n');
            let template = compile_str(joined_template_string.as_str());
            let input_cfg = match from_str(input_cfg_str) {
                Ok(cfg) => cfg,
                Err(e) => {
                    return Err(ConfigurationError::ParsingError(e));
                }
            };
            Ok((template, input_cfg))
        },
        Err(io_error) => {
            Err(ConfigurationError::Inaccessible(io_error))
        }
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

/// Look up a format entry by name - helper.
fn lookup_format_entry(cfg: &Config,
                       timers: &mut Vec<Timer>,
                       fifos: &mut Vec<Fifo>,
                       name: &str)
    -> ConfigResult<()> {
    let workaround_name = format!("workaround.{}", name);
    let t = try!(get_child(&cfg, workaround_name.as_str(), "type"));
    if t == "timer" {
        let path = try!(get_child(&cfg, workaround_name.as_str(), "command"));
        let path2 = format!("{}.seconds", workaround_name);
        let path3 = format!("{}.minutes", workaround_name);
        let path4 = format!("{}.hours", workaround_name);
        let seconds = if let Some(d) = cfg.lookup_integer32(path2.as_str()) {
            d as i64
        } else {
            cfg.lookup_integer64_or(path2.as_str(), 0)
        };
        let minutes = if let Some(d) = cfg.lookup_integer32(path3.as_str()) {
            d as i64
        } else {
            cfg.lookup_integer64_or(path3.as_str(), 0)
        };
        let hours = if let Some(d) = cfg.lookup_integer32(path4.as_str()) {
            d as i64
        } else {
            cfg.lookup_integer64_or(path4.as_str(), 0)
        };
        let duration = Duration::hours(hours) +
            Duration::minutes(minutes) + Duration::seconds(seconds);

        if duration > Duration::seconds(0) {
            timers.push(Timer {
                period: duration,
                command: String::from(path),
                name: String::from(name)
            });
            Ok(())
        } else {
            Err(ConfigurationError::IllegalValue(String::from(name)))
        }
    } else if t == "fifo" {
        let path = try!(get_child(&cfg, &workaround_name, "fifo_path"));
        fifos.push(Fifo {
            path: try!(parse_path(path)),
            name: String::from(name),
        });
        Ok(())
    } else {
        Err(ConfigurationError::IllegalType(String::from(name)))
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

