extern crate crypto;
extern crate dirs;
extern crate filetime;
extern crate getopts;
extern crate serde_json;

use std::{env, io::Error, io::Read, process::exit, thread::sleep, time::Duration};
use std::fs::{File, Metadata, copy, metadata};
use std::path::{Path, PathBuf};
use std::time::{SystemTime};
use crypto::{digest::Digest, sha1::Sha1};
use getopts::Options;
use filetime::{FileTime, set_file_times};
use serde_json::{Value as JSONValue};

const FILES_THE_SAME: &'static str = "Files are the same! Not updating.";
const BUFFER_SIZE: usize = 8096; // 8 KB
const SLEEP_TIME: u64 = 10;

enum SetupError {
    MalformedCLI(String),
    ConfigLoadError(String),
}

impl SetupError {
    pub fn to_string(&self) -> String {
        use SetupError::*;
        match self {
            MalformedCLI(s) => s.clone(),
            ConfigLoadError(s) => s.clone(),
        }
    }
}

impl From<std::io::Error> for SetupError {
    fn from(err: std::io::Error) -> Self {
        SetupError::ConfigLoadError(err.to_string())
    }
}

impl From<serde_json::Error> for SetupError {
    fn from(err: serde_json::Error) -> Self {
        SetupError::ConfigLoadError(err.to_string())
    }
}

fn error(string: &str) -> ! {
    println!("\x1b[1m\x1b[91mERROR: {}\x1b[0m", string);
    exit(1);
}

fn usage() { 
    println!(r#"staticsync [OPTIONS]

OPTIONS:
-c, --config CONFIG Path to a configuration file. Will use .staticsync.json in your home folder if unspecified.
-d, --delay SECONDS Delay time between each check
-s, --size SIZE     Hashing buffer size, in bytes (default: 8 KB, 8096)
-v, --verbose       Show more information when synching
-o, --once          Only run sync once"#);
}

fn setup() -> Result<(bool, bool, usize, JSONValue, Duration), SetupError> {
    let args: Vec<String> = env::args().collect();
    let config_file: String;
    let sleep_time: Duration;
    let buffer_size: usize;
    let verbose: bool;
    let once: bool;

    let mut opts = Options::new();
    opts.optopt("c", "config", "", "");
    opts.optopt("d", "delay", "", "");
    opts.optopt("s", "size", "", "");
    opts.optflag("v", "verbose", "");
    opts.optflag("o", "once", "");
    opts.optflag("h", "help", "");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(f) => { return Err(SetupError::MalformedCLI(f.to_string())); }
    };

    if matches.opt_present("help") {
        usage();
        exit(0);
    }

    verbose = matches.opt_present("verbose");
    once = matches.opt_present("once");

    buffer_size = match matches.opt_str("size") {
        Some(s) => {
            if let Ok(n) = s.parse::<usize>() {
                n
            } else {
                error("Cache size invalid.");
            }
        }

        None => BUFFER_SIZE
    };

    config_file = match matches.opt_str("config") {
        Some(s) => s,
        None => {
            let mut buf: PathBuf = dirs::home_dir().unwrap();
            buf.push(".staticsync.json");
            if !buf.as_path().is_file() {
                return Err(SetupError::ConfigLoadError("Missing config file".to_string()))
            }

            buf.to_str().unwrap().to_string()
        }
    };

    sleep_time = Duration::from_secs(match matches.opt_str("delay") {
        Some(s) => {
            let secs: Option<u64> = s.parse::<u64>().ok();
            match secs {
                Some(s) => s,
                None => return Err(SetupError::MalformedCLI("Invalid interval number".to_string()))
            }
        },
        None => SLEEP_TIME
    });

    println!("Loading config \"{}\"...", config_file);
    let file = File::open(config_file)?;

    let value: JSONValue = serde_json::from_reader(file)?;
    let same_error = |x: &str| { SetupError::ConfigLoadError(format!("Duplicated path: {}", x)) };
    let dir_error = |x: &str| { SetupError::ConfigLoadError(format!("Path \"{}\" is a directory!", x)) };
    let abs_error = |x: &str| { SetupError::ConfigLoadError(format!("Path must be absolute: {}", x)) };
    let exs_error = |x: &str| { SetupError::ConfigLoadError(format!("File \"{}\" does not exist!", x)) };

    {
        // Validate paths
        let files: &Vec<JSONValue> = value.get("files").unwrap().as_array().unwrap();
        for entry in files {
            let buf: Vec<PathBuf> = entry.as_array().unwrap().iter().take(2).map(|x| PathBuf::from(x.as_str().unwrap())).collect();
            let path: Vec<&Path> = buf.iter().map(|x| x.as_path()).collect();

            // Check if paths are absolute
            if !path[0].is_absolute() { return Err(abs_error(path[0].to_str().unwrap())); }
            if !path[1].is_absolute() { return Err(abs_error(path[1].to_str().unwrap())); }

            // Check if paths are directories
            if path[0].is_dir() { return Err(dir_error(path[0].to_str().unwrap())); }
            if path[1].is_dir() { return Err(dir_error(path[1].to_str().unwrap())); }

            // Check if paths are duplicated
            if path[0] == path[1] { return Err(same_error(path[0].to_str().unwrap())); }

            // Check if files exist
            // TODO: Check for both files not existing instead (sync)
            if !path[0].exists() { return Err(exs_error(path[0].to_str().unwrap())); }
            if !path[1].exists() { return Err(exs_error(path[1].to_str().unwrap())); }
        }
    }

    Ok((verbose, once, buffer_size, value, sleep_time))
}

fn calculate_hash(buffer_size: usize, path: &str) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let mut buf: Vec<u8> = Vec::with_capacity(buffer_size);
    unsafe { buf.set_len(buffer_size); }
    let mut hasher = Sha1::new();

    loop {
        let n = file.read(&mut buf)?;
        hasher.input(&buf[..n]);
        if n == 0 || n < buf.len() { break }
    }
    
    Ok(hasher.result_str())
}

fn sync(verbose: bool, buffer_size: usize, config: &JSONValue) {
    use std::cmp::Ordering;

    println!("Checking...");
    let files = config.get("files").unwrap().as_array().unwrap();

    for entry in files {
        let path: Vec<&str> = entry.as_array().unwrap()
            .iter().take(2).map(|x| x.as_str().unwrap()).collect();
        // TODO: Check for either file existing so it can be created on the other end
        let meta: Vec<Metadata> = path
            .iter().map(|x| metadata(x).unwrap()).collect();
        let ftime: Vec<FileTime> = meta.iter()
            .map(|x| FileTime::from_last_modification_time(&x)).collect();

        if verbose {
            println!("{} vs {}", path[0], path[1]);
            println!("\tmtime: {} --- {}", ftime[0], ftime[1]);
        }

        let (newest, oldest) = {
            match ftime[0].cmp(&ftime[1]) {
                Ordering::Greater => (0, 1),
                Ordering::Less => (1, 0),
                Ordering::Equal => {
                    if verbose { println!("\t{}", FILES_THE_SAME); }
                    continue;
                }
            }
        };

        let hash: Vec<String> = path.iter().map(|x| calculate_hash(buffer_size, x).unwrap()).collect();
        let atime = FileTime::from_system_time(SystemTime::now());
        if verbose { 
            println!("\t#{} is newer. Checking hashes...", newest+1);
            println!("\t{} vs {}", hash[0], hash[1]);
        }

        if hash[0] != hash[1] {
            if verbose { println!("\tReplacing #{} with #{}", newest+1, oldest+1); }
            copy(path[newest], path[oldest]).expect("Make sure you have permissions to copy!");
            set_file_times(path[oldest], atime, ftime[newest]).expect("Make sure you have permission to modify timestamps!");
            if !verbose { println!("Updated {}", path[oldest]); }
        } else {
            if verbose { println!("\t{}", FILES_THE_SAME); }
            // Update filetime in that case so we don't waste time hashing again.
            set_file_times(path[oldest], atime, ftime[newest]).expect("Make sure you have permission to modify timestamps!");
        }
    }
}

fn main() {
    let (verbose, once, buffer_size, config, sleep_time) = match setup() {
        Ok(v) => v,
        Err(e) => error(&e.to_string())
    };

    loop {
        sync(verbose, buffer_size, &config);
        if once { break }
        sleep(sleep_time);
    }
}
