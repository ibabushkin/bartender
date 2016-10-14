extern crate config;
extern crate getopts;
extern crate libc;
extern crate time;

use getopts::Options;

use std::env;
use std::io::Write;
use std::path::Path;

#[macro_use]
pub mod bartender;
pub mod poll;

use bartender::Configuration;

/// Main function.
///
/// Read in command line arguments, parse options and configuration file.
/// Then run the deamon according to the configuration data found.
fn main() {
    // collect CLI args
    let args: Vec<String> = env::args().collect();

    // set up option parsing
    let mut opts = Options::new();
    opts.optopt("c", "config", "set config file name", "FILE");
    opts.optflag("h", "help", "print this help menu");

    // match on args and decide what to do
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };
    if matches.opt_present("h") {
        let desc = format!("usage: {} [options]", args[0]);
        print!("{}", opts.usage(&desc));
        return;
    }

    // obtain and parse config file
    let config = if let Some(path) = matches.opt_str("c") {
        Configuration::from_config_file(Path::new(path.as_str()))
    } else if let Some(mut dir) = env::home_dir() {
        dir.push(".bartenderrc");
        match dir.canonicalize() {
            Ok(path) => Configuration::from_config_file(path.as_path()),
            Err(err) => panic!("error: {}", err),
        }
    } else {
        panic!("no config file could be determined!");
    };

    match config {
        Ok(config) => {
            err!("obtained config: {:?}", config);
            config.run()
        },
        Err(e) => err!("error reading config: {}", e),
    }
}
