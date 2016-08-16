extern crate config;
extern crate getopts;

use config::reader::from_file;

use getopts::Options;

use std::env;
use std::path::Path;

fn main() {
    // options parsing
    let args: Vec<String> = env::args().collect();
    let mut opts = Options::new();
    opts.optopt("c", "config", "set config file name", "CONFIG FILE");
    opts.optflag("h", "help", "print this help menu");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };
    if matches.opt_present("h") {
        let desc = format!("Usage: {} FILE [options]", args[0]);
        print!("{}", opts.usage(&desc));
        return;
    }

    // obtaining and parsing config file
    let config = if let Some(path) = matches.opt_str("c") {
        from_file(Path::new(path.as_str()))
    } else if let Some(mut home_dir) = env::home_dir() {
        home_dir.push(".bartenderrc");
        from_file(home_dir.as_path())
    } else {
        panic!("no config file could be determined!");
    };
}
