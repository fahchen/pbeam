#!/usr/bin/env python3
"""Measure the Terra Counter start time and key latency in a pseudo-terminal.

Usage: terra_latency.py <command> [arguments...]

The command runs on a fresh 80 by 24 pseudo-terminal with TERM=xterm-256color
and a UTF-8 locale. The harness waits for the first frame, then sends the keys
j, j, k, and q two seconds apart, and prints the time from the start of the
command to the first frame and the time from each key to the first byte that the
command writes after that key. The first frame time is the time of the read that
shows the frame, not the end of a quiet window, so it is not late by the quiet
time. `scripts/terra_latency.sh` runs this harness for the debug binary, the
release binary, and pinned ERTS, and writes the result to a file.

A host that denies /dev/ptmx, for example an agent sandbox, cannot run this
harness.
"""

import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

COLS, ROWS = 80, 24
FIRST_FRAME_BOUND = 300.0
KEY_BOUND = 60.0
QUIET = 0.4
KEY_GAP = 2.0

cmd = sys.argv[1:]
if not cmd:
    sys.exit(__doc__)

pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    os.environ["TERM"] = "xterm-256color"
    os.environ["LANG"] = "en_US.UTF-8"
    os.environ["LC_ALL"] = "en_US.UTF-8"
    os.execvp(cmd[0], cmd)

start = time.time()


def visible(data):
    """Drop the control sequences, so the text of a frame can be matched."""
    text = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", bytes(data))
    return re.sub(rb"\x1b[()][0-9A-Za-z]", b"", text)


def read_frame(bound):
    """Read until the first frame, the end of the command, or `bound` seconds.

    Return the bytes, the time of the read that shows the frame, and whether
    the command ended. The time is that read, so a frame that the command draws
    and then leaves alone is not late by the quiet time that `read_until`
    waits for.
    """
    out = bytearray()
    deadline = time.time() + bound
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return out, None, True
        if not chunk:
            return out, None, True
        out += chunk
        if b"Count:" in visible(out):
            return out, time.time(), False
    return out, None, False


def read_until(bound, quiet=0.0):
    """Read until `bound` seconds pass, or until the stream is quiet for `quiet`."""
    out = bytearray()
    deadline = time.time() + bound
    last = time.time()
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if ready:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return out, True
            if not chunk:
                return out, True
            out += chunk
            last = time.time()
        elif quiet and time.time() - last > quiet:
            break
    return out, False


def read_first(bound):
    """Read until the first byte arrives, or until `bound` seconds pass."""
    deadline = time.time() + bound
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return b"", True
        if not chunk:
            return b"", True
        return chunk, False
    return b"", False


seen, frame_at, ended = read_frame(FIRST_FRAME_BOUND)
if frame_at is None:
    if ended:
        print("the command ended before the first frame")
    else:
        print("no first frame within %.0f s" % FIRST_FRAME_BOUND)
    tail = visible(seen)[-400:].decode("utf-8", "replace")
    if tail.strip():
        print("last output:", tail)
    sys.exit(1)

print("first frame after %6.2f s" % (frame_at - start), flush=True)

previous = None
for key in "jjkq":
    if previous is not None:
        delay = KEY_GAP - (time.time() - previous)
        if delay > 0:
            time.sleep(delay)
    _, ended = read_until(2.0, QUIET)
    if ended:
        print("the command ended before the key %s" % key, flush=True)
        break
    sent = time.time()
    previous = sent
    try:
        os.write(fd, key.encode())
    except OSError:
        print("the command ended before the key %s" % key, flush=True)
        break
    answer, ended = read_first(KEY_BOUND)
    if answer:
        print("key %s answered after %6.3f s" % (key, time.time() - sent), flush=True)
        read_until(2.0, QUIET)
    elif ended:
        print("key %s ended the command" % key, flush=True)
    else:
        print("key %s got no answer within %.0f s" % (key, KEY_BOUND), flush=True)
    if ended:
        break

try:
    os.close(fd)
except OSError:
    pass
try:
    os.kill(pid, 15)
except OSError:
    pass
