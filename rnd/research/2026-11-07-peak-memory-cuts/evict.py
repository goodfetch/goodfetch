"""peak-cuts: drops a file's clean pages from the page cache
(posix_fadvise DONTNEED, no root needed), so the next run pages the
executable in through its own faults, as after a reboot.

usage: evict.py <file>...
"""
import os
import sys

for p in sys.argv[1:]:
    fd = os.open(p, os.O_RDONLY)
    os.fdatasync(fd)
    os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
    os.close(fd)
