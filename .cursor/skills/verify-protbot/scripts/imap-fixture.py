#!/usr/bin/env python3
"""Loopback IMAP and SMTP stand-in for a verify run.

Speaks the subset Proton Mail Bridge speaks that src/bridge.rs implements:
greeting, CAPABILITY without STARTTLS, LOGIN, SELECT, UID FETCH (headers and
BODY.PEEK[]), LIST, LOGOUT, and a small SMTP dialogue without STARTTLS.
The bridge password is read from PROTON_BRIDGE_PASSWORD and is never logged.
"""

from __future__ import annotations

import argparse
import os
import signal
import socketserver
import threading
import time

HEADER = b"From: Owner <owner@proton.me>\r\nSubject: Verify mailbox\r\n"
BODY = (
    b"From: Owner <owner@proton.me>\r\n"
    b"Subject: Verify mailbox\r\n"
    b"\r\n"
    b"hello from the loopback mailbox\r\n"
)

LOG_LOCK = threading.Lock()
STOP = False


def log_line(path: str, text: str) -> None:
    line = text.replace("\n", " ").replace("\r", " ")
    with LOG_LOCK:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(line + "\n")


class ImapHandler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(30)
        self._send(b"* OK fixture\r\n")
        while True:
            line = self.rfile.readline()
            if not line:
                return
            self._one(line)

    def _one(self, line: bytes) -> None:
        text = line.decode("utf-8", "replace").rstrip("\r\n")
        tag, _, rest = text.partition(" ")
        if not tag:
            return
        upper = rest.upper()
        password = self.server.password
        if upper.startswith("LOGIN"):
            if password and f'"{password}"' in rest:
                log_line(self.server.log_path, "IMAP LOGIN ok")
                self._send(f"{tag} OK\r\n".encode())
            else:
                log_line(self.server.log_path, "IMAP LOGIN rejected")
                self._send(f"{tag} NO\r\n".encode())
            return
        if upper.startswith("CAPABILITY"):
            log_line(self.server.log_path, "IMAP CAPABILITY")
            self._send(f"* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n".encode())
            return
        if "FETCH" in upper:
            log_line(self.server.log_path, f"IMAP {rest}")
            if "BODY.PEEK[]" in upper or "BODY[]" in upper:
                self._fetch_body(tag)
            else:
                self._fetch_headers(tag)
            return
        if upper.startswith("LIST"):
            log_line(self.server.log_path, "IMAP LIST")
            self._send(
                (
                    '* LIST (\\HasNoChildren) "/" "INBOX"\r\n'
                    '* LIST (\\HasNoChildren) "/" "Sent"\r\n'
                    f"{tag} OK\r\n"
                ).encode()
            )
            return
        verb = rest.split(" ", 1)[0] if rest else "EMPTY"
        log_line(self.server.log_path, f"IMAP {verb}")
        self._send(f"{tag} OK\r\n".encode())

    def _fetch_headers(self, tag: str) -> None:
        literal = f"* 1 FETCH (UID 7 FLAGS () BODY[HEADER.FIELDS (FROM SUBJECT)] {{{len(HEADER)}}}\r\n".encode()
        self._send(literal + HEADER + b")\r\n" + f"{tag} OK\r\n".encode())

    def _fetch_body(self, tag: str) -> None:
        literal = f"* 1 FETCH (UID 7 BODY[] {{{len(BODY)}}}\r\n".encode()
        self._send(literal + BODY + f"\r\n{tag} OK\r\n".encode())

    def _send(self, payload: bytes) -> None:
        self.wfile.write(payload)
        self.wfile.flush()


class SmtpHandler(socketserver.StreamRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(30)
        self._send(b"220 fixture ESMTP\r\n")
        while True:
            line = self.rfile.readline()
            if not line:
                return
            text = line.decode("utf-8", "replace").rstrip("\r\n")
            upper = text.upper()
            if upper.startswith("EHLO") or upper.startswith("HELO"):
                log_line(self.server.log_path, "SMTP EHLO")
                self._send(b"250-fixture\r\n250 AUTH PLAIN\r\n")
            elif upper.startswith("AUTH"):
                log_line(self.server.log_path, "SMTP AUTH")
                self._send(b"235 OK\r\n")
            elif upper.startswith("MAIL FROM"):
                log_line(self.server.log_path, "SMTP MAIL")
                self._send(b"250 OK\r\n")
            elif upper.startswith("RCPT TO"):
                log_line(self.server.log_path, "SMTP RCPT")
                self._send(b"250 OK\r\n")
            elif upper.startswith("DATA"):
                log_line(self.server.log_path, "SMTP DATA")
                self._send(b"354 Go\r\n")
                self._read_data()
                self._send(b"250 OK\r\n")
            elif upper.startswith("QUIT"):
                log_line(self.server.log_path, "SMTP QUIT")
                self._send(b"221 Bye\r\n")
                return
            else:
                verb = upper.split(" ", 1)[0] if upper else "EMPTY"
                log_line(self.server.log_path, f"SMTP {verb}")
                self._send(b"250 OK\r\n")

    def _read_data(self) -> None:
        chunks: list[bytes] = []
        while True:
            line = self.rfile.readline()
            if not line or line in (b".\r\n", b".\n"):
                break
            chunks.append(line)
        blob = b"".join(chunks)
        subject = ""
        for line in blob.splitlines():
            if line.lower().startswith(b"subject:"):
                subject = line.split(b":", 1)[1].strip().decode("utf-8", "replace")
                break
        log_line(self.server.log_path, f"SMTP DATA-END bytes={len(blob)} subject={subject}")
        dest = getattr(self.server, "smtp_copy", "")
        if dest:
            with open(dest, "wb") as handle:
                handle.write(blob)

    def _send(self, payload: bytes) -> None:
        self.wfile.write(payload)
        self.wfile.flush()


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True
    password = ""
    log_path = ""
    smtp_copy = ""


def serve(port: int, handler: type) -> Server:
    server = Server(("127.0.0.1", port), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def main() -> None:
    parser = argparse.ArgumentParser(description="loopback IMAP/SMTP fixture for protbot verify")
    parser.add_argument("--imap-port", type=int, default=0)
    parser.add_argument("--smtp-port", type=int, default=0)
    parser.add_argument("--ready-file", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--smtp-copy", default="")
    args = parser.parse_args()
    password = os.environ.get("PROTON_BRIDGE_PASSWORD", "")
    if not password:
        raise SystemExit("PROTON_BRIDGE_PASSWORD is unset; the fixture will not invent one")

    imap = serve(args.imap_port, ImapHandler)
    smtp = serve(args.smtp_port, SmtpHandler)
    imap.password = password
    smtp.password = password
    imap.log_path = args.log
    smtp.log_path = args.log
    smtp.smtp_copy = args.smtp_copy

    ready = (
        f"imap_port={imap.server_address[1]}\n"
        f"smtp_port={smtp.server_address[1]}\n"
    )
    with open(args.ready_file, "w", encoding="utf-8") as handle:
        handle.write(ready)

    def stop(_signum: int, _frame: object) -> None:
        global STOP
        STOP = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while not STOP:
        time.sleep(0.2)
    imap.shutdown()
    smtp.shutdown()


if __name__ == "__main__":
    main()
