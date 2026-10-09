import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import private_run


class DisplayReply(unittest.TestCase):
    def reply(self, data):
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, data)
            os.close(write_fd)
            write_fd = None
            return private_run.display_number(read_fd, timeout=1)
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)

    def test_complete_number(self):
        self.assertEqual(self.reply(b"0\n"), ":0")

    def test_split_reply_waits_for_newline(self):
        read_fd, write_fd = os.pipe()
        consumed = threading.Event()
        result = []
        original_read = os.read

        def read(fd, count):
            data = original_read(fd, count)
            if fd == read_fd and data == b"4":
                consumed.set()
            return data

        def write():
            try:
                os.write(write_fd, b"4")
                if not consumed.wait(1):
                    result.append(False)
                    return
                os.write(write_fd, b"2\n")
                result.append(True)
            finally:
                os.close(write_fd)

        writer = threading.Thread(target=write)
        writer.start()
        try:
            with patch.object(private_run.os, "read", side_effect=read):
                self.assertEqual(private_run.display_number(read_fd, timeout=2), ":42")
        finally:
            os.close(read_fd)
            writer.join(timeout=2)
        self.assertFalse(writer.is_alive())
        self.assertEqual(result, [True])

    def test_eof_and_malformed_replies(self):
        for value in [b"", b"42", b"\n", b"-1\n", b"0x1\n", b"1\n2", b"9" * 65 + b"\n"]:
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                self.reply(value)

    def test_deadline_without_reply(self):
        read_fd, write_fd = os.pipe()
        try:
            with self.assertRaises(RuntimeError):
                private_run.display_number(read_fd, timeout=0.01)
        finally:
            os.close(read_fd)
            os.close(write_fd)

    def test_spawn_failure_closes_both_pipe_descriptors(self):
        descriptors = []
        original_pipe = os.pipe

        def pipe():
            pair = original_pipe()
            descriptors.extend(pair)
            return pair

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "candidate"
            binary.write_bytes(b"private candidate")
            profile = root / "profile"
            profile.mkdir()
            (profile / "settings.cfg").write_text("fov 90\n")
            with patch.object(private_run.os, "pipe", side_effect=pipe), patch.object(
                private_run.subprocess, "Popen", side_effect=FileNotFoundError
            ):
                result = private_run.run(binary, profile, root / "evidence", [])
        self.assertEqual(result["result"], "FAIL")
        self.assertEqual(result["remaining_owned_pids"], [])
        self.assertTrue(result["owner_profile_unchanged"])
        self.assertTrue(result["candidate_unchanged"])
        self.assertEqual(len(descriptors), 2)
        for descriptor in descriptors:
            with self.assertRaises(OSError):
                os.fstat(descriptor)


if __name__ == "__main__":
    unittest.main()
