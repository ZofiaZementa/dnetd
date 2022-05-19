use anyhow::Result;
use libc;
use std::{os::unix::io::RawFd, ptr};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpliceError {
    #[error("invalid argument")]
    InvalidArgument,
    #[error("broken pipe")]
    BrokenPipe,
}

pub fn splice(fd_in: RawFd, fd_out: RawFd) -> Result<usize, SpliceError> {
    let res = unsafe {
        libc::splice(
            fd_in,
            ptr::null_mut(),
            fd_out,
            ptr::null_mut(),
            libc::ssize_t::MAX as libc::size_t,
            libc::SPLICE_F_MOVE,
        )
    };
    if res <= -1 {
        match unsafe { *libc::__errno_location() } {
            libc::EAGAIN => panic!("SPLICE_F_NONBLOCK was specified in flags or one of the file descriptors had been marked as nonblocking (O_NONBLOCK), and the operation would block"),
            libc::EBADF => panic!("One or both file descriptors are not valid, or do not have proper read-write mode"),
            libc::EINVAL => Err(SpliceError::InvalidArgument),
            libc::ENOMEM => panic!("Out of memory"),
            libc::ESPIPE => unreachable!("Either off_in or off_out was not NULL, but the corresponding file descriptor refers to a pipe"),
            libc::EPIPE => Err(SpliceError::BrokenPipe),
            e => unreachable!("Some error type that shouldn't occur occured: {:?}", e),
        }
    } else {
        Ok(res as usize)
    }
}

// enum PollEvent {
//     In = libc::POLLIN as isize,
//     Pri = libc::POLLPRI as isize,
//     Out = libc::POLLOUT as isize,
//     ReadHangup = libc::POLLRDHUP as isize,
//     Error = libc::POLLERR as isize,
//     Hangup = libc::POLLHUP as isize,
//     Invalid = libc::POLLNVAL as isize,
//     ReadBand = libc::POLLRDBAND as isize,
//     WriteBand = libc::POLLWRBAND as isize,
// }

// #[derive(Debug, Clone, Copy)]
// struct PollEvents(i16);

// impl PollEvents {
//     fn new() -> Self {
//         PollEvents(0)
//     }

//     fn add_event(&mut self, event: PollEvent) -> () {
//         self.0 |= event as i16
//     }

//     fn remove_event(&mut self, event: PollEvent) -> () {
//         self.0 &= !(event as i16)
//     }

//     fn contains(&self, event: PollEvent) -> bool {
//         self.0 & (event as i16) != 0
//     }
// }

// struct PollItem(libc::pollfd);

// impl PollItem {
//     fn new(fd: RawFd, events: PollEvents) -> Self {
//         PollItem(libc::pollfd {
//             fd,
//             events: events.0,
//             revents: 0,
//         })
//     }

//     fn set_events(&mut self, events: PollEvents) -> () {
//         self.0.events = events.0
//     }

//     fn events(&self) -> PollEvents {
//         PollEvents(self.0.events)
//     }

//     fn revents(&self) -> PollEvents {
//         PollEvents(self.0.revents)
//     }
// }

// #[derive(Error, Debug)]
// enum PollError {
//     #[error("A signal occurred before any requested event; see signal(7)")]
//     Interrupt,
//     #[error("items was empty")]
//     ItemsEmpty,
//     #[error("items.len() exceeds {}", libc::RLIMIT_NOFILE)]
//     ItemsTooBig,
// }

// fn poll(items: &mut Vec<PollItem>, timeout: Duration) -> Result<usize, PollError> {
//     if !items.is_empty() {
//         return Err(PollError::ItemsEmpty);
//     }
//     let res = unsafe {
//         libc::poll(
//             &mut items.first_mut().unwrap().0 as *mut libc::pollfd,
//             items.len() as libc::nfds_t,
//             timeout.as_millis() as libc::c_int,
//         )
//     };
//     if res <= -1 {
//         match unsafe { *libc::__errno_location() } {
//             libc::EFAULT => unreachable!("fds points outside the process's accessible address space.  The array given as argument was not contained in the calling program's address space."),
//             libc::EINTR => Err(PollError::Interrupt),
//             libc::EINVAL => Err(PollError::ItemsTooBig),
//             libc::ENOMEM => panic!("Unable to allocate memory for kernel data structures."),
//             e => unreachable!("Some error type that shouldn't occur occured: {:?}", e),
//         }
//     } else {
//         Ok(res as usize)
//     }
// }
