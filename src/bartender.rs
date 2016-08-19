//! Configuration Parser module.
//!
//! Presents types and functions to read in, represent and interpret data
//! found in configuration files for the software.
use config::error::ConfigError;
use config::reader::from_file;
use config::types::{Config,ScalarValue,Setting,Value};

use std::path::{Path,PathBuf};
use std::sync::mpsc;
use std::thread;

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
                    command: PathBuf::from(path),
                }));
            } else if t == "fifo" {
                let path = try!(get_child(&cfg, &name, "fifo_path"));
                fifos.push((index, Fifo {
                    path: PathBuf::from(path),
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

/// An error that occured during setup.
#[derive(Debug)]
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
    seconds: u32,
    command: PathBuf,
}

impl Timer {
    pub fn run(&self, index: usize, tx: mpsc::Sender<(usize, String)>) {

    }
}

/// A FIFO source.
#[derive(Debug)]
pub struct Fifo {
    path: PathBuf,
}

impl Fifo {
    pub fn run(&self, index: usize, tx: mpsc::Sender<(usize, String)>) {

    }
}

/// An Output buffer.
#[derive(Debug)]
pub struct Buffer {
    format: Vec<String>,
}

impl Buffer {
    fn set(&mut self, index: usize, value: String) {
        self.format[index] = value;
    }

    fn output(&self) {
        println!("{}", self.format.join(""));
    }
}
