"""Read complete Gomocup responses against a single monotonic deadline."""

import os
import select
import time


class ResponseReader:
    def __init__(self):
        self.buffer = bytearray()

    def lines(self, stream, deadline):
        while True:
            remaining = None if deadline is None else deadline - time.monotonic()
            if remaining is not None and remaining <= 0:
                raise TimeoutError("engine response deadline expired")
            end = self.buffer.find(b"\n")
            if end >= 0:
                line = self.buffer[:end].decode("utf-8", errors="replace").strip()
                del self.buffer[:end + 1]
                yield line
                continue
            readable, _, _ = select.select([stream], [], [], remaining)
            if not readable:
                raise TimeoutError("engine response deadline expired")
            chunk = os.read(stream.fileno(), 8192)
            if not chunk:
                if self.buffer:
                    line = self.buffer.decode("utf-8", errors="replace").strip()
                    self.buffer.clear()
                    yield line
                return
            self.buffer.extend(chunk)
