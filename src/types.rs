use config::error::ConfigError;
use config::reader::from_file;
use config::types::{Config,ScalarValue,Setting,Value};

use std::path::{Path,PathBuf};

#[derive(Debug)]
pub struct Configuration {
    buffer: Buffer,
    timers: Vec<(usize, Timer)>,
    fifos: Vec<(usize, Fifo)>,
}

impl Configuration {
    pub fn from_config_file(file: &Path) -> ConfigResult<Configuration> {
        let cfg = try!(parse_config_file(file));

        let mut format_string = Vec::new();
        let mut entries = Vec::new();

        let format =
            if let Some(&Value::List(ref l)) = cfg.lookup("format") {
                l
            } else {
                return Err(ConfigurationError::MissingFormat);
            };

        for entry in format {
            match entry {
                &Value::Svalue(ScalarValue::Str(ref s)) =>
                    format_string.push(s.clone()),
                &Value::Group(ref s) =>
                    if let Some(&Setting {
                            name: _,
                            value: Value::Svalue(ScalarValue::Str(ref name)),
                        }) = s.get("name") {
                        entries.push((name.clone(), format_string.len()));
                        format_string.push(String::new());
                    } else {
                        return Err(ConfigurationError::IllegalFormat);
                    },
                _ => return Err(ConfigurationError::IllegalFormat),
            }
        }

        let mut timers = Vec::new();
        let mut fifos = Vec::new();

        for (ref name, index) in entries {
            let t = try!(get_child(&cfg, &name, "type"));
            if t == "timer" {
                let path = try!(get_child(&cfg, &name, "command_path"));
                timers.push((index, Timer {
                    seconds: get_seconds(&cfg, &name),
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

        Ok(Configuration {
            buffer: Buffer { entries: format_string },
            timers: timers,
            fifos: fifos,
        })
    }
}

#[derive(Debug)]
pub enum ConfigurationError {
    ParsingError(ConfigError),
    MissingFormat,
    IllegalFormat,
    MissingChild(String, String),
    IllegalType(String),
}

pub type ConfigResult<T> = Result<T, ConfigurationError>;

fn parse_config_file(file: &Path) -> ConfigResult<Config> {
    match from_file(file) {
        Ok(cfg) => Ok(cfg),
        Err(e) => Err(ConfigurationError::ParsingError(e)),
    }
}

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

fn get_seconds(cfg: &Config, name: &str) -> u32 {
    cfg.lookup_integer32_or(format!("{}.seconds", name).as_str(), 1) as u32
}

#[derive(Debug)]
pub struct Timer {
    seconds: u32,
    command: PathBuf,
}

#[derive(Debug)]
pub struct Fifo {
    path: PathBuf,
}

#[derive(Debug)]
pub struct Buffer {
    entries: Vec<String>,
}
