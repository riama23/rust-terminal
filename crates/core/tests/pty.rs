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

#[test]
fn spawn_in_directory_and_foreground_group() {
    let dir = std::env::temp_dir();
    let pty = Pty::spawn_in(
        "/bin/sh",
        &["-c", "pwd; sleep 1"],
        no_env(),
        Some(&dir),
        80,
        24,
    )
    .unwrap();
    let (mut reader, writer, mut child) = pty.split().unwrap();
    // Mientras corre `sleep`, el grupo en primer plano es el de la shell (sin
    // control de trabajos, sh no crea grupos nuevos): existe y es un pid.
    assert!(writer.foreground_pgrp().is_some());
    let mut out = String::new();
    let mut buf = [0u8; 1024];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        out.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    child.wait().unwrap();
    let expected = dir.canonicalize().unwrap();
    assert!(out.contains(expected.to_str().unwrap()), "salida: {out:?}");
}

#[test]
fn hangup_terminates_the_shell() {
    let pty = Pty::spawn("/bin/sh", &["-c", "sleep 30"], no_env(), 80, 24).unwrap();
    let (_reader, _writer, mut child) = pty.split().unwrap();
    rust_terminal_core::hangup(child.id());
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
fn control_handle_works_while_a_write_is_blocked() {
    // El hijo no lee: una escritura grande llena el búfer de la PTY y se bloquea.
    // `exec`: el proceso que recibe SIGHUP es el propio `sleep`.
    let pty = Pty::spawn("/bin/sh", &["-c", "exec sleep 3"], no_env(), 80, 24).unwrap();
    let (_reader, writer, mut child) = pty.split().unwrap();
    let control = writer.try_clone().unwrap();
    let mut writer = writer;
    // El hilo bloqueado no se espera: solo importa que el otro manejador responda.
    std::thread::spawn(move || {
        let _ = std::io::Write::write_all(&mut writer, &vec![b'x'; 4 << 20]);
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    // El otro manejador sigue respondiendo.
    let start = std::time::Instant::now();
    control.resize(100, 40).unwrap();
    assert!(control.foreground_pgrp().is_some());
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    // No se espera al hijo: en macOS, un proceso que cierra la terminal con
    // la cola de entrada llena se queda atascado al salir.
    let _ = child.kill();
}
