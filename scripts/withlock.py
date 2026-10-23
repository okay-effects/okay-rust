#!/usr/bin/env python3
"""Run a command holding JMH's lock ($TMPDIR/jmh.lock, the file JMH itself takes): other sessions measure on this
machine, and a run while theirs goes on measures both. Waits for the lock, runs, releases.
usage: scripts/withlock.py <command> [args...]"""
import fcntl, os, subprocess, sys, tempfile

path = os.path.join(tempfile.gettempdir(), "jmh.lock")
with open(path, "a") as f:
    fcntl.lockf(f, fcntl.LOCK_EX)
    sys.exit(subprocess.call(sys.argv[1:]))
