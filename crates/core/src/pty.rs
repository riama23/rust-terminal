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
        Pty::spawn_in(program, args, env, None, cols, rows)
    }

    /// Como [`Pty::spawn`], pero arrancando en el directorio `cwd` si se da.
    pub fn spawn_in<I, K, V>(
        program: impl AsRef<OsStr>,
        args: &[&str],
        env: I,
        cwd: Option<&std::path::Path>,
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
        if let Some(dir) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }

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

    /// Separa la sesión en lectura, escritura y proceso hijo, para usarlos
    /// desde hilos distintos. Ambos lados comparten el mismo maestro.
    pub fn split(self) -> io::Result<(PtyReader, PtyWriter, Child)> {
        let writer = PtyWriter {
            master: self.master.try_clone()?,
        };
        let reader = PtyReader {
            master: self.master,
        };
        Ok((reader, writer, self.child))
    }
}

/// Lee del maestro. Cuando el hijo ha cerrado el esclavo, macOS puede
/// devolver `EIO` en lugar de 0; se trata como EOF.
fn read_master(master: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    match master.read(buf) {
        Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => Ok(0),
        other => other,
    }
}

impl Read for Pty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_master(&mut self.master, buf)
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

/// Lado de lectura de una PTY separada con [`Pty::split`].
pub struct PtyReader {
    master: File,
}

impl Read for PtyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_master(&mut self.master, buf)
    }
}

/// Lado de escritura de una PTY separada con [`Pty::split`]: teclas, pegado,
/// respuestas a consultas y cambios de tamaño.
pub struct PtyWriter {
    master: File,
}

impl PtyWriter {
    /// Comunica el nuevo tamaño al hijo (`TIOCSWINSZ`, que envía `SIGWINCH`).
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        tcsetwinsize(&self.master, winsize(cols, rows))?;
        Ok(())
    }
}

impl PtyWriter {
    /// Grupo de procesos en primer plano de la terminal. Si no es el de la
    /// shell, hay un programa en marcha (vim, Claude Code, un `sleep`…).
    pub fn foreground_pgrp(&self) -> Option<u32> {
        rustix::termios::tcgetpgrp(&self.master)
            .ok()
            .map(|p| p.as_raw_nonzero().get() as u32)
    }
}

/// Envía SIGHUP a un proceso (la shell de una sesión que se cierra), como
/// hace una terminal al cerrar una ventana.
pub fn hangup(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::HUP);
    }
}

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.master.flush()
    }
}
