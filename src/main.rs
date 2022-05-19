mod debuggee;
mod pidfile;

use anyhow::{anyhow, Result, Error};
use clap::Arg;
use debuggee::DebuggeeSet;
use log::info;
use std::{
    net::{IpAddr, TcpListener},
    os::unix::prelude::CommandExt,
    process,
    time::Duration,
};

fn main() -> Result<()> {
    let args = clap::Command::new("dnetd")
        .arg(
            Arg::new("address")
                .short('a')
                .long("address")
                .takes_value(true)
                .default_value("0.0.0.0")
                .help("Address on which to listen"),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .takes_value(true)
                .default_value("1024")
                .help("Port on which to listen"),
        )
        .arg(
            Arg::new("user")
                .short('u')
                .long("user")
                .takes_value(true)
                .help("User which to use [default: current]"),
        )
        .arg(
            Arg::new("timeout")
                .short('t')
                .long("timeout")
                .takes_value(true)
                .help("Timeout of socket in seconds [default: None]"),
        )
        .arg(
            Arg::new("verbosity")
                .short('v')
                .multiple_occurrences(true)
                .help("Increase message verbosity"),
        )
        .arg(
            Arg::new("command")
                .takes_value(true)
                .multiple_values(true)
                .required(true)
                .help("The command which to execute"),
        )
        .get_matches();

    stderrlog::new()
        .module(module_path!())
        .verbosity(args.occurrences_of("verbosity") as usize + 1)
        .init()?;

    let port: u16 = args
        .value_of("port")
        .unwrap()
        .parse()
        .map_err(|_| anyhow!("Invalid Argument: port must be a value betwenn 1 and 65536"))?;
    let ip: IpAddr =
        args.value_of("address").unwrap().parse().map_err(|_| {
            anyhow!("Invalid Argument: address must be a valid Ipv4 or Ipv6 address")
        })?;
    let mut cmd_args = args.values_of("command").unwrap();
    let mut pidfile = pidfile::PidFile::init()?;
    let mut cmd = process::Command::new(cmd_args.next().unwrap());
    cmd.args(cmd_args);
    if args.is_present("user") {
        let uid: u32 = args
            .value_of("user")
            .unwrap()
            .parse()
            .map_err(|_| anyhow!("Invalid Argument: user must be a valid uid"))?;
        cmd.uid(uid);
    }
    // map argument to duration and propagate error outwards
    let timeout = args.value_of("timeout").map_or(anyhow::Ok(None), |t| {
        Ok(Some(Duration::from_secs(t.parse().map_err(|_| {
            anyhow!("Invaid Argument: must be a valid integer")
        })?)))
    })?;

    let mut debuggees = DebuggeeSet::new(cmd);
    let debuggee_pid = debuggees.spool_debuggee()?;
    info!("Started debuggee, PID {}", debuggee_pid);
    pidfile.set_pid(debuggee_pid)?;
    let listener = TcpListener::bind((ip, port))?;

    for stream in listener.incoming() {
        debuggees.cleanup()?;
        match stream {
            Ok(stream) => {
                // error shouldn't occur because we know the spool isn't empty
                info!("Incoming connection from {}", stream.peer_addr()?);
                debuggees.start_debuggee(stream, timeout).unwrap();
                let debuggee_pid = debuggees.spool_debuggee()?;
                info!("Started debuggee, PID {}", debuggee_pid);
                pidfile.set_pid(debuggee_pid)?;
            }
            Err(e) => return Err(Error::from(e)),
        }
    }
    Ok(())
}
