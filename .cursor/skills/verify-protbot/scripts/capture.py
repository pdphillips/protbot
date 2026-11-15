#!/usr/bin/env python3
"""Copy tmux pane output to a log with no buffering. Invoked by pipe-pane."""

import os
import sys

# os.read returns whatever tmux has already written. BufferedReader.read(4096)
# waits until 4096 bytes or EOF, so short JSON lines would sit forever.
path = sys.argv[1]
fd = sys.stdin.buffer.fileno()
with open(path, "ab", buffering=0) as handle:
    while True:
        chunk = os.read(fd, 4096)
        if not chunk:
            break
        handle.write(chunk)
