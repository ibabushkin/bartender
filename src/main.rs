extern crate getopts;
extern crate libc;
extern crate mustache;
extern crate time;
extern crate toml;

use getopts::Options;

use std::env;
use std::io::Write;
use std::path::Path;
use std::process::exit;

#[macro_use]
pub mod bartender;
pub mod mkfifo;
pub mod poll;

use bartender::Config;

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
        Err(f) => {
            err!("error: parsing args failed: {}", f.to_string());
            exit(1);
        },
    };
    if matches.opt_present("h") {
        let desc = format!("usage: {} [options]", args[0]);
        print!("{}", opts.usage(&desc));
        return;
    }

    // obtain and parse config file
    let config = if let Some(path) = matches.opt_str("c") {
        Config::from_config_file(Path::new(path.as_str()))
    } else if let Some(mut dir) = env::home_dir() {
        dir.push(".bartenderrc");
        match dir.canonicalize() {
            Ok(path) => Config::from_config_file(path.as_path()),
            Err(err) => {
                err!("error: {}", err);
                exit(1);
            },
        }
    } else {
        err!("no config file could be determined!",);
        exit(1);
    };

    match config {
        Ok(config) => {
            err!("obtained config: {:?}", config);
            config.run()
        },
        Err(e) => {
            err!("error: reading config failed: {}", e);
            exit(1);
        },
    }
}
