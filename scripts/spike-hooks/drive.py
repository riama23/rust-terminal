#!/usr/bin/env python3
"""Lanza `claude` interactivo en un pseudo-terminal y vuelca lo que muestra.

Solo observa: nunca envía Intro ni respuestas, así que un prompt de permiso
queda sin contestar y la sesión se mata al terminar el tiempo.

Uso: drive.py SETTINGS "prompt" [segundos] [salida.txt]
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time

settings, prompt = sys.argv[1], sys.argv[2]
seconds = int(sys.argv[3]) if len(sys.argv) > 3 else 60
out_path = sys.argv[4] if len(sys.argv) > 4 else "/tmp/spike-drive.txt"

pid, fd = pty.fork()
if pid == 0:
    os.execvp("claude", ["claude", "--settings", settings,
                         "--permission-mode", "default", prompt])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 140, 0, 0))

buf = b""
end = time.time() + seconds
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.5)
    if r:
        try:
            data = os.read(fd, 65536)
        except OSError:
            break
        if not data:
            break
        buf += data
os.kill(pid, signal.SIGTERM)
time.sleep(0.5)
try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass

text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]", b"", buf)
with open(out_path, "wb") as f:
    f.write(text)
print(f"{len(buf)} bytes capturados -> {out_path}")
