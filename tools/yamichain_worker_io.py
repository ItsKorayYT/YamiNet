"""Deadline-bounded persistent worker exchange. Never resets worker durable state."""
import json
import os
import select
import time


def exchange(process, command, timeout=15):
    deadline=time.monotonic()+timeout
    pending=(json.dumps(command)+'\n').encode()
    received=bytearray()
    read_fd=process.stdout.fileno();write_fd=process.stdin.fileno()
    os.set_blocking(read_fd,False);os.set_blocking(write_fd,False)
    try:
        while True:
            remaining=deadline-time.monotonic()
            if remaining<=0:raise TimeoutError('private worker response deadline')
            readable,writable,_=select.select([read_fd],[write_fd] if pending else [],[],remaining)
            if writable:
                try:pending=pending[os.write(write_fd,pending):]
                except BlockingIOError:pass
            if readable:
                try:chunk=os.read(read_fd,65536)
                except BlockingIOError:continue
                if not chunk:raise RuntimeError('private worker stopped')
                received.extend(chunk)
                if len(received)>16*1024*1024:raise ValueError('private worker response byte limit')
                if b'\n' in received:
                    line,extra=received.split(b'\n',1)
                    if extra or pending:raise ValueError('private worker response framing')
                    return json.loads(line)
    except Exception:
        process.terminate()
        raise
