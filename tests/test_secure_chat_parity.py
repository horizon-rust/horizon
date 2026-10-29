import json
import unittest
from unittest import mock

from tools import test_secure_chat_parity as parity


class StatusSocket:
    def __init__(self, chunks):
        self.chunks = list(chunks)
        self.eof_reads = 0
        self.closed = False

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.closed = True

    def sendall(self, data):
        pass

    def recv(self, size):
        if not self.chunks:
            self.eof_reads += 1
            if self.eof_reads > 1:
                raise AssertionError("status reader called recv again after EOF")
            return b""
        chunk = self.chunks[0]
        result = chunk[:size]
        if len(result) == len(chunk):
            self.chunks.pop(0)
        else:
            self.chunks[0] = chunk[len(result):]
        return result


class SecureChatStatusTests(unittest.TestCase):
    def setUp(self):
        self.expected = {
            "enforcesSecureChat": True,
            "description": {"text": "distinctive status 雪"},
            "players": {"online": 3, "max": 17},
        }
        text = json.dumps(self.expected, ensure_ascii=False).encode()
        self.payload = b"\0" + parity.lab.varint(len(text)) + text
        self.wire = parity.lab.varint(len(self.payload)) + self.payload

    def test_valid_distinctive_json(self):
        connection = StatusSocket([self.wire])
        with mock.patch.object(parity.socket, "create_connection", return_value=connection):
            self.assertEqual(parity.status(25565), self.expected)
        self.assertTrue(connection.closed)
        self.assertEqual(connection.eof_reads, 0)

    def test_header_eof_is_a_retryable_connection_error(self):
        connection = StatusSocket([])
        with mock.patch.object(parity.socket, "create_connection", return_value=connection):
            with self.assertRaisesRegex(ConnectionError, "status length"):
                parity.status(25565)
        self.assertTrue(connection.closed)
        self.assertEqual(connection.eof_reads, 1)

    def test_partial_body_eof_stops_after_the_first_empty_read(self):
        connection = StatusSocket([parity.lab.varint(len(self.payload)), self.payload[:7]])
        with mock.patch.object(parity.socket, "create_connection", return_value=connection):
            with self.assertRaisesRegex(ConnectionError, "status body"):
                parity.status(25565)
        self.assertTrue(connection.closed)
        self.assertEqual(connection.eof_reads, 1)

    def test_fragmented_valid_reads_keep_the_complete_json(self):
        connection = StatusSocket([self.wire[i:i + 1] for i in range(len(self.wire))])
        with mock.patch.object(parity.socket, "create_connection", return_value=connection):
            self.assertEqual(parity.status(25565), self.expected)
        self.assertTrue(connection.closed)
        self.assertEqual(connection.eof_reads, 0)


if __name__ == "__main__":
    unittest.main()
