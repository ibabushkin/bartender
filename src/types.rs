use config::error::ConfigError;
use config::reader::from_file;
use config::types::Config;

use std::path::{Path,PathBuf};

pub struct Configuration {
    format: Formatter,
    timers: Vec<Timer>,
    fifos: Vec<Fifo>,
}

impl Configuration {
    pub fn from_config_file(file: &Path) -> ConfigResult<Configuration> {
        let cfg = try!(parse_config_file(file));
        let formatter = try!(get_formatter(&cfg));

        let mut timers = Vec::new();
        let mut fifos = Vec::new();

        for name in formatter.get_names() {
            let t = try!(get_child(&cfg, name, "type"));
            if t == "timer" {
                let path = try!(get_child(&cfg, name, "command_path"));
                timers.push(Timer {
                    seconds: get_seconds(&cfg, name),
                    command: PathBuf::from(path),
                });
            } else if t == "fifo" {
                let path = try!(get_child(&cfg, name, "fifo_path"));
                fifos.push(Fifo {
                    path: PathBuf::from(path),
                });
            } else {
                return Err(
                    ConfigurationError::IllegalType(String::from(*name))
                );
            }
        }

        Ok(Configuration {
            format: formatter,
            timers: timers,
            fifos: fifos,
        })
    }
}

pub enum ConfigurationError {
    ParsingError(ConfigError),
    MissingFormat,
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

fn get_formatter(cfg: &Config) -> ConfigResult<Formatter> {
    if let Some(format) = cfg.lookup_str("format") {
        Ok(Formatter::new(format))
    } else {
        Err(ConfigurationError::MissingFormat)
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

pub struct Timer {
    seconds: u32,
    command: PathBuf,
}

pub struct Fifo {
    path: PathBuf,
}

pub struct Formatter {
    format_string: String,
}

impl Formatter {
    fn new(string: &str) -> Formatter {
        Formatter {
            format_string: String::from(string),
        }
    }

    fn get_names(&self) -> &[&str] {
        &[]
    }
}
