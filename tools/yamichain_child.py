#!/usr/bin/env python3
"""Exec a supervised Linux child with parent-death termination and race checking."""
import ctypes
import os
import signal
import sys

if sys.platform != 'linux' or len(sys.argv) < 3:
    raise SystemExit('supervised native services require Linux parent-death handling')
parent = int(sys.argv[1])
libc = ctypes.CDLL(None, use_errno=True)
libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
libc.prctl.restype = ctypes.c_int
if libc.prctl(1, signal.SIGTERM, 0, 0, 0) != 0:
    raise OSError(ctypes.get_errno(), 'cannot establish supervised child lifetime')
# The parent could have exited between fork and prctl. Never continue orphaned.
if os.getppid() != parent:
    raise SystemExit('supervisor exited before child initialization')
os.execv(sys.argv[2], sys.argv[2:])
