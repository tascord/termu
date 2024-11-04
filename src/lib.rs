use nix::{
    fcntl::{
        fcntl,
        FcntlArg::{self, F_SETFD},
        FdFlag, OFlag,
    },
    libc::{close, ioctl, setsid, TIOCSCTTY, TIOCSWINSZ},
    pty::{grantpt, posix_openpt, ptsname_r, unlockpt, PtyMaster, Winsize},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Stdin},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, ExitStatus},
};

pub struct Terminal {
    owner: PtyMaster,
    pub stdin: Option<IoFd>,
    pub stdout: Option<IoFd>,
    child: Child,
}

impl Terminal {
    pub fn open(command: &mut Command) -> io::Result<Self> {
        let owner = posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY)?;
        grantpt(&owner)?;
        unlockpt(&owner)?;

        let mut flags = FdFlag::from_bits_retain(fcntl(owner.as_raw_fd(), FcntlArg::F_GETFD)?);
        flags |= FdFlag::FD_CLOEXEC;

        fcntl(owner.as_raw_fd(), F_SETFD(flags))?;

        let pup = OpenOptions::new()
            .read(true)
            .write(true)
            .open(ptsname_r(&owner)?)?;

        command.stdin(pup.try_clone()?);
        command.stdout(pup.try_clone()?);
        command.stderr(pup.try_clone()?);

        unsafe {
            let o_fd = owner.as_raw_fd();
            command.pre_exec(move || {
                if close(o_fd) != 0 || setsid() < 0 || ioctl(0, TIOCSCTTY.into(), 1) != 0 {
                    return Err(io::Error::last_os_error());
                }

                Ok(())
            });
        }

        Ok(Self {
            stdin: Some(IoFd(owner.as_fd().try_clone_to_owned()?)),
            stdout: Some(IoFd(owner.as_fd().try_clone_to_owned()?)),
            child: command.spawn()?,
            owner,
        })
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub fn resize(&self, size: Winsize) -> io::Result<()> {
        match unsafe { ioctl(self.owner.as_raw_fd(), TIOCSWINSZ, &size) != 0 } {
            true => Err(io::Error::last_os_error()),
            false => Ok(()),
        }
    }
}

pub struct IoFd(OwnedFd);
impl IoFd {
    pub fn exists(&self) -> bool {
        fcntl(self.0.as_raw_fd(), FcntlArg::F_GETFD).unwrap_or(-1) != -1
    }
}

impl io::Read for IoFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.exists() {
            true => File::from(self.0.try_clone()?).read(buf),
            _ => Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "File Descriptor Closed",
            )),
        }
    }
}

impl io::Write for IoFd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.exists() {
            true => File::from(self.0.try_clone()?).write(buf),
            _ => Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "File Descriptor Closed",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.exists() {
            true => File::from(self.0.try_clone()?).flush(),
            _ => Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "File Descriptor Closed",
            )),
        }
    }
}

impl From<Stdin> for IoFd {
    fn from(value: Stdin) -> Self {
        Self(value.as_fd().try_clone_to_owned().unwrap())
    }
}
