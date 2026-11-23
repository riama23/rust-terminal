//! Pseudoterminal: crea el par maestro/esclavo y lanza el proceso hijo.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use rustix::fs::OFlags;
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

/// Una sesión: el lado maestro de la PTY y el proceso que corre en el esclavo.
pub struct Pty {
    master: File,
    child: Child,
}

fn winsize(cols: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Pty {
    /// Lanza `program` en una PTY nueva de `cols`×`rows`.
    ///
    /// El hijo hereda el entorno del proceso con `TERM=xterm-256color` y
    /// `COLORTERM=truecolor`; `env` se aplica encima.
    pub fn spawn<I, K, V>(
        program: impl AsRef<OsStr>,
        args: &[&str],
        env: I,
        cols: u16,
        rows: u16,
    ) -> io::Result<Pty>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        // macOS no acepta `O_CLOEXEC` en `posix_openpt`: se pone después, para
        // que el hijo no herede el maestro.
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;

        // El esclavo solo vive en el padre hasta el `spawn`: si quedara abierto
        // aquí, el maestro nunca vería EOF cuando el hijo termine.
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags((OFlags::NOCTTY | OFlags::CLOEXEC).bits() as i32)
            .open(OsStr::from_bytes(name.as_bytes()))?;
        tcsetwinsize(&slave, winsize(cols, rows))?;

        let mut cmd = Command::new(program);
        cmd.args(args)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .envs(env)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));

        // SAFETY: entre `fork` y `exec` solo se hacen llamadas al sistema
        // seguras para señales asíncronas (`setsid` e `ioctl`), sin reservar memoria.
        unsafe {
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                // std ya ha conectado el esclavo como stdin (fd 0).
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                Ok(())
            });
        }
        let child = cmd.spawn()?;

        Ok(Pty {
            master: File::from(master),
            child,
        })
    }

    /// Comunica el nuevo tamaño al hijo (`TIOCSWINSZ`, que envía `SIGWINCH`).
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        tcsetwinsize(&self.master, winsize(cols, rows))?;
        Ok(())
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn master(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }
}

impl Read for Pty {
    /// Lee del maestro. Cuando el hijo ha cerrado el esclavo, macOS puede
    /// devolver `EIO` en lugar de 0; se trata como EOF.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.read(buf) {
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => Ok(0),
            other => other,
        }
    }
}

impl Write for Pty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.master.flush()
    }
}
