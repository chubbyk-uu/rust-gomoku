import os
import time
import unittest

from scripts.gomocup_response import ResponseReader


class GomocupResponseTests(unittest.TestCase):
    def test_message_and_partial_coordinate_still_time_out(self):
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, "rb") as stream, os.fdopen(write_fd, "wb", buffering=0) as writer:
            writer.write(b"MESSAGE thinking\n7,")
            lines = ResponseReader().lines(stream, time.monotonic() + 0.1)
            self.assertEqual(next(lines), "MESSAGE thinking")
            with self.assertRaises(TimeoutError):
                next(lines)

    def test_multiple_lines_in_one_read_survive_response_boundary(self):
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, "rb") as stream, os.fdopen(write_fd, "wb", buffering=0) as writer:
            writer.write(b"OK\r\nMESSAGE ready\n7,7\n")
            reader = ResponseReader()
            self.assertEqual(next(reader.lines(stream, None)), "OK")
            lines = reader.lines(stream, time.monotonic() + 1)
            self.assertEqual(next(lines), "MESSAGE ready")
            self.assertEqual(next(lines), "7,7")
