extern crate crypto;
extern crate dirs;
extern crate failure;
extern crate filetime;
extern crate getopts;
extern crate serde_json;

use std::{env, io::Read, process::exit, thread::sleep, time::Duration};
use std::fs::{File, Metadata, metadata};
use std::io::{BufReader};
use std::path::{Path, PathBuf};
use crypto::{digest::Digest, sha1::Sha1};
use getopts::Options;
use failure::Error;
use filetime::FileTime;
use serde_json::{Value as JSONValue};

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
    println!(r#"staticsync [-c CONFIG_PATH]

OPTIONS:
-c CONFIG\tPath to a configuration file. Will use .staticsync.json in your home folder if this is not specified.
"#);
}

fn setup() -> Result<(JSONValue, Duration), SetupError> {
    let args: Vec<String> = env::args().collect();
    let config_file: String;
    let sleep_time: Duration;

    let mut opts = Options::new();
    opts.optopt("c", "config", "Config file", "PATH");
    opts.optopt("t", "time", "Interval between checks in seconds.", "SECONDS");
    opts.optopt("h", "help", "Find help", "SECONDS");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(f) => { return Err(SetupError::MalformedCLI(f.to_string())); }
    };

    if matches.opt_present("help") {
        usage();
        exit(0);
    }

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

    sleep_time = Duration::from_secs(match matches.opt_str("time") {
        Some(s) => {
            let secs: Option<u64> = s.parse::<u64>().ok();
            match secs {
                Some(s) => s,
                None => return Err(SetupError::MalformedCLI("Invalid interval number".to_string()))
            }
        },
        None => 10
    });

    println!("Loading config \"{}\"...", config_file);
    let file = File::open(config_file)?;

    let value: JSONValue = serde_json::from_reader(file)?;
    let abs_error = |x: &str| { SetupError::ConfigLoadError(format!("Path must be absolute: {}", x)) };
    let exs_error = |x: &str| { SetupError::ConfigLoadError(format!("File \"{}\" does not exist!", x)) };

    {
        // Check if paths are absolute and if they exist
        let files: &Vec<JSONValue> = value.get("files").unwrap().as_array().unwrap();
        for entry in files {
            let buf: Vec<PathBuf> = entry.as_array().unwrap().iter().take(2).map(|x| PathBuf::from(x.as_str().unwrap())).collect();
            let path: Vec<&Path> = buf.iter().map(|x| x.as_path()).collect();
            if !path[0].is_absolute() { return Err(abs_error(path[0].to_str().unwrap())); }
            if !path[1].is_absolute() { return Err(abs_error(path[1].to_str().unwrap())); }
            // TODO: Check for both files not existing instead (see sync)
            if !path[0].exists() { return Err(exs_error(path[0].to_str().unwrap())); }
            if !path[1].exists() { return Err(exs_error(path[1].to_str().unwrap())); }
        }
    }

    Ok((value, sleep_time))
}

fn calculate_hash(path: &str) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let mut buf = [0u8; 1024*8];
    let mut hasher = Sha1::new();

    loop {
        let n = file.read(&mut buf)?;
        hasher.input(&buf[..n]);
        if n == 0 || n < buf.len() { break }
    }
    
    Ok(hasher.result_str())
}

fn sync(config: &JSONValue) {
    use std::cmp::Ordering;

    println!("\nChecking...");
    let files = config.get("files").unwrap().as_array().unwrap();

    for entry in files {
        let path: Vec<&str> = entry.as_array().unwrap()
            .iter().take(2).map(|x| x.as_str().unwrap()).collect();
        // TODO: Check for either file existing so it can be created on the other end
        let meta: Vec<Metadata> = path
            .iter().map(|x| metadata(x).unwrap()).collect();
        let ftime: Vec<FileTime> = meta.iter()
            .map(|x| FileTime::from_last_modification_time(&x)).collect();

        println!("{} vs {}", path[0], path[1]);
        println!("\tmtime: {} --- {}", ftime[0], ftime[1]);

        let (newest, oldest) = {
            match ftime[0].cmp(&ftime[1]) {
                Ordering::Greater => (0, 1),
                Ordering::Less => (1, 0),
                Ordering::Equal => {
                    println!("\tFiles are the same!");
                    continue;
                }
            }
        };

        println!("\t#{} is newer. Checking hashes...", newest+1);
        let hash: Vec<String> = path.iter().map(|x| calculate_hash(x).unwrap()).collect();
        println!("{:?}", hash);
        if hash[0] != hash[1] {
            println!("\tReplacing #{} with #{}", newest+1, oldest+1);
            // TODO: File copy
        } else {
            println!("\tFiles are the same! Not updating.");
        }
    }
}

fn main() {
    let (config, sleep_time) = match setup() {
        Ok(v) => v,
        Err(e) => error(&e.to_string())
    };

    loop {
        sync(&config);
        sleep(sleep_time);
    }
}
