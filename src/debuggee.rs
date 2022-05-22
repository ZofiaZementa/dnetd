mod splice;

use anyhow::{anyhow, ensure, Error, Result};
use log::{debug, error, trace};
use std::{
    collections::HashMap,
    mem,
    net::{Shutdown, TcpStream},
    os::unix::prelude::AsRawFd,
    process,
    sync::{Arc, RwLock},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Debug)]
enum DebuggeeCommsAux {
    Splice {
        incoming: JoinHandle<()>,
        outgoing: JoinHandle<()>,
    },
    Timeout(Option<Instant>),
}

#[derive(Debug)]
struct DebuggeeComms {
    stream: TcpStream,
    timeout: Option<Duration>,
    aux: DebuggeeCommsAux,
}

impl DebuggeeComms {
    pub fn to_timeout(&mut self) -> Result<(JoinHandle<()>, JoinHandle<()>)> {
        ensure!(
            matches!(
                self.aux,
                DebuggeeCommsAux::Splice {
                    incoming: _,
                    outgoing: _
                }
            ),
            "CommsAux was not Splice"
        );
        match mem::replace(
            &mut self.aux,
            DebuggeeCommsAux::Timeout(self.timeout.map(|t| Instant::now() + t)),
        ) {
            DebuggeeCommsAux::Splice { incoming, outgoing } => Ok((incoming, outgoing)),
            _ => panic!("Found Pattern that should be impossible"),
        }
    }

    pub fn is_timedout(&self) -> bool {
        match self.aux {
            DebuggeeCommsAux::Timeout(Some(t)) => {
                let now = Instant::now();
                trace!("Now: {:?} Timeout: {:?}", now, t);
                now >= t
            },
            _ => false,
        }
    }

    pub fn timeout(&mut self) -> Result<()> {
        match self.aux {
            DebuggeeCommsAux::Timeout(Some(t)) => {
                if Instant::now() >= t {
                    trace!("socket timed out, shutting down");
                    self.stream.shutdown(Shutdown::Both).map_err(Error::from)
                } else {
                    Err(anyhow!("Wasn't timed out yet"))
                }
            }
            _ => Err(anyhow!("CommsAux was not Timout")),
        }
    }
}

#[derive(Debug)]
struct InnerDebuggee {
    proc: process::Child,
    comms: Option<DebuggeeComms>,
}

impl Drop for InnerDebuggee {
    fn drop(&mut self) {
        // ignore error, killing it shoudn't throw error
        let _ = self.proc.kill();
    }
}

// WARNING: Debuggee should not be used outside of this
// This only works if once the Zombie is fetched it is instantly taken out of
// the DebuggeeSet. Otherwise there might be two equal processes in the map.
// Do not use this to actually compare Debuggees
#[derive(Debug, Clone)]
struct Debuggee {
    inner: Arc<RwLock<InnerDebuggee>>,
}

impl Debuggee {
    pub fn spool(proc: process::Child) -> Self {
        Debuggee {
            inner: Arc::new(RwLock::new(InnerDebuggee { proc, comms: None })),
        }
    }

    pub fn start(&mut self, stream: TcpStream, timeout: Option<Duration>) -> Result<()> {
        let mut inner = self.inner.write().unwrap();

        let self_incoming = self.clone();
        let stdin_fd = inner.proc.stdin.as_ref().unwrap().as_raw_fd();
        let stream_fd_incoming = stream.as_raw_fd();
        let incoming = thread::Builder::new()
            .name("incoming".to_string())
            .spawn(move || {
                trace!("started incoming thread");
                loop {
                    match splice::splice(stream_fd_incoming, stdin_fd) {
                        Err(splice::SpliceError::BrokenPipe) => {
                            // Don't shutdown socket, there might still be stuff outgoing
                            debug!("broken pipe incoming");
                            break;
                        }
                        Err(_) => unreachable!("some error that shouldn't occoured"),
                        Ok(0) => {
                            // Shutdown socket since the other end is closed
                            debug!("EOI incoming, closing socket");
                            let mut inner = self_incoming.inner.write().unwrap();
                            let comms = inner.comms.as_mut().unwrap();
                            // Dont care if socket is already shutdown
                            let _ = comms.stream.shutdown(Shutdown::Both);
                            let (_, outgoing) = comms.to_timeout().unwrap();
                            // close fds so child exits
                            drop(inner.proc.stdin.take());
                            drop(inner.proc.stdout.take());
                            drop(inner.proc.stderr.take());
                            if let Err(e) = outgoing.join() {
                                error!("Error joining outgoing thread: {:?}", e);
                            } else {
                                trace!("outgoing thread joined");
                            }
                            break;
                        }
                        _ => (),
                    }
                }
                trace!("shutting down incoming thread");
            })?;
        let self_outgoing = self.clone();
        let stdout_fd = inner.proc.stdout.as_ref().unwrap().as_raw_fd();
        let stream_fd_outgoing = stream.as_raw_fd();
        let outgoing = thread::Builder::new()
            .name("outgoing".to_string())
            .spawn(move || {
                trace!("started outgoing thread");
                loop {
                    match splice::splice(stdout_fd, stream_fd_outgoing) {
                        Err(splice::SpliceError::BrokenPipe) => {
                            debug!("broken pipe outgoing, closing socket");
                            let mut inner = self_outgoing.inner.write().unwrap();
                            let comms = inner.comms.as_mut().unwrap();
                            // Dont care if socket is already shutdown
                            let _ = comms.stream.shutdown(Shutdown::Both);
                            let (incoming, _) = comms.to_timeout().unwrap();
                            // close fds so child exits
                            drop(inner.proc.stdin.take());
                            drop(inner.proc.stdout.take());
                            drop(inner.proc.stderr.take());
                            if let Err(e) = incoming.join() {
                                error!("Error joining incoming thread: {:?}", e);
                            } else {
                                trace!("incoming thread joined");
                            }
                            break;
                        }
                        Err(_) => unreachable!("some error that shouldn't occoured"),
                        Ok(0) => {
                            debug!("EOI outgoing");
                            break;
                        }
                        _ => (),
                    }
                }
                trace!("shutting down outgoing thread");
            })?;

        (*inner).comms = Some(DebuggeeComms {
            stream,
            timeout,
            aux: DebuggeeCommsAux::Splice { incoming, outgoing },
        });
        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<Option<process::ExitStatus>> {
        let mut inner = self.inner.write().unwrap();
        if inner.comms.is_some() && inner.comms.as_ref().unwrap().is_timedout() {
            inner.comms.as_mut().unwrap().timeout()?;
            inner.proc.try_wait().map_err(Error::from)
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct DebuggeeSet {
    debuggees: HashMap<u32, Debuggee>,
    cmd: process::Command,
    spool: Option<Debuggee>,
}

impl DebuggeeSet {
    pub fn new(mut cmd: process::Command) -> Self {
        cmd.stdin(process::Stdio::piped());
        cmd.stdout(process::Stdio::piped());
        cmd.stderr(process::Stdio::null());
        DebuggeeSet {
            debuggees: HashMap::new(),
            cmd,
            spool: Option::None,
        }
    }

    pub fn spool_debuggee(&mut self) -> Result<u32> {
        ensure!(self.spool.is_none(), "Spool is not empty");
        let child = self.cmd.spawn()?;
        let pid = child.id();
        self.spool = Some(Debuggee::spool(child));
        Ok(pid)
    }

    pub fn start_debuggee(&mut self, stream: TcpStream, timeout: Option<Duration>) -> Result<()> {
        ensure!(self.spool.is_some(), "Spool is empty");
        let mut debuggee = self.spool.take().unwrap();
        debuggee.start(stream, timeout)?;
        let pid = debuggee.inner.read().unwrap().proc.id();
        self.debuggees.insert(pid, debuggee);
        Ok(())
    }

    // pub fn check_spool(&self) -> bool {
    //     self.spool.is_some()
    // }

    // pub fn clear_spool(&mut self) {
    //     // actual destruction etc. is handled by drop
    //     self.spool = None;
    // }

    pub fn cleanup(&mut self) -> Result<HashMap<u32, process::ExitStatus>> {
        let mut exited = HashMap::new();
        for (pid, dres) in self.debuggees.iter_mut().map(|(p, d)| (p, d.cleanup())) {
            trace!("Tried cleaning up debuggee {}, got {:?}", pid, dres);
            if let Some(d) = dres? {
                debug!("Debuggee {} exited with {}", pid, d);
                exited.insert(*pid, d);
            }
        }
        for pid in exited.keys() {
            self.debuggees.remove(pid);
        }
        Ok(exited)
    }
}
