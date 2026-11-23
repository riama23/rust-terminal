use std::io::Read;

use rust_terminal_core::Pty;

fn no_env() -> std::iter::Empty<(&'static str, &'static str)> {
    std::iter::empty()
}

fn read_to_eof(pty: &mut Pty) -> String {
    let mut out = Vec::new();
    pty.read_to_end(&mut out).expect("leer de la PTY");
    pty.child().wait().expect("esperar al hijo");
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn child_output_reaches_master() {
    let mut pty = Pty::spawn("/bin/echo", &["hola"], no_env(), 80, 24).unwrap();
    assert!(read_to_eof(&mut pty).contains("hola"));
}

#[test]
fn initial_size_and_term_env_are_set() {
    let mut pty = Pty::spawn(
        "/bin/sh",
        &["-c", "stty size; echo \"$TERM $COLORTERM\""],
        no_env(),
        100,
        30,
    )
    .unwrap();
    let out = read_to_eof(&mut pty);
    assert!(out.contains("30 100"), "salida: {out:?}");
    assert!(out.contains("xterm-256color truecolor"), "salida: {out:?}");
}

#[test]
fn resize_is_seen_by_child() {
    // El hijo espera a que le llegue una línea antes de consultar el tamaño.
    let mut pty = Pty::spawn("/bin/sh", &["-c", "read x; stty size"], no_env(), 80, 24).unwrap();
    pty.resize(120, 40).unwrap();
    std::io::Write::write_all(&mut pty, b"\n").unwrap();
    let out = read_to_eof(&mut pty);
    assert!(out.contains("40 120"), "salida: {out:?}");
}

#[test]
fn child_has_controlling_terminal() {
    let mut pty = Pty::spawn(
        "/bin/sh",
        &["-c", "tty && ps -o tty= -p $$"],
        no_env(),
        80,
        24,
    )
    .unwrap();
    let out = read_to_eof(&mut pty);
    assert!(out.contains("/dev/ttys"), "salida: {out:?}");
    assert!(!out.contains("??"), "sin terminal de control: {out:?}");
}

#[test]
fn split_reader_and_writer_work_from_different_threads() {
    let pty = Pty::spawn(
        "/bin/sh",
        &["-c", "read x; echo \"got $x\"; stty size"],
        no_env(),
        80,
        24,
    )
    .unwrap();
    let (mut reader, mut writer, mut child) = pty.split().unwrap();
    let handle = std::thread::spawn(move || {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        String::from_utf8_lossy(&out).into_owned()
    });
    writer.resize(90, 33).unwrap();
    std::io::Write::write_all(&mut writer, b"ping\n").unwrap();
    child.wait().unwrap();
    drop(writer);
    let out = handle.join().unwrap();
    assert!(out.contains("got ping"), "salida: {out:?}");
    assert!(out.contains("33 90"), "salida: {out:?}");
}
