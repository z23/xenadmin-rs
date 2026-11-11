#!/usr/bin/env python3
"""Close every session of one directory subject on a lab pool.

The live harness uses this to stand in for another administrator ending a user's
sessions. XenCenter offers that as "Logout User"; the client under test has no such
operation, so the harness cannot do it through the client. The script logs in as the lab's
root account, calls session.logout_subject_identifier and logs out again.

    force_logout.py HOST SHA256_FINGERPRINT SUBJECT_IDENTIFIER

The password is read from the file named by XENADMIN_LAB_PASSWORD_FILE and is never taken
from the command line or printed. Nothing is sent to a host whose certificate does not
match the fingerprint. XAPI never closes a local superuser's session this way, so the
account used here keeps its own.
"""
import hashlib
import http.client
import json
import os
import ssl
import sys


class Refused(Exception):
    """The host was not the pinned one, or it answered the call with an error."""


def normalized(fingerprint):
    """A SHA-256 fingerprint as 64 lower-case hexadecimal digits, separators removed."""
    digits = "".join(c for c in fingerprint.lower() if c not in ": ")
    if len(digits) != 64 or any(c not in "0123456789abcdef" for c in digits):
        raise ValueError("the fingerprint is not a SHA-256 digest")
    return digits


def matches(certificate, fingerprint):
    """Whether a DER certificate has the pinned SHA-256 fingerprint."""
    return hashlib.sha256(certificate).hexdigest() == normalized(fingerprint)


def request(method, params):
    return json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})


def result(reply):
    """The result of a JSON-RPC reply. An error is reported by its code alone, because the
    rest of an error can repeat the parameters of the call."""
    document = json.loads(reply)
    if "error" in document:
        error = document["error"]
        code = error.get("message") if isinstance(error, dict) else None
        raise Refused(f"the host refused the call: {code or 'unknown error'}")
    return document["result"]


def call(host, fingerprint, method, params, timeout=30):
    """One call on a connection of its own, sent only after the certificate matched."""
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE  # trust is the fingerprint checked below
    connection = http.client.HTTPSConnection(host, 443, timeout=timeout, context=context)
    try:
        connection.connect()
        if not matches(connection.sock.getpeercert(binary_form=True), fingerprint):
            raise Refused("the host's certificate does not match the fingerprint")
        connection.request(
            "POST", "/jsonrpc", request(method, params), {"Content-Type": "application/json"}
        )
        return result(connection.getresponse().read())
    finally:
        connection.close()


def force_logout(host, fingerprint, subject, password):
    session = call(host, fingerprint, "session.login_with_password",
                   ["root", password, "1.0", "xenadmin-rs-lab-tooling"])
    try:
        call(host, fingerprint, "session.logout_subject_identifier", [session, subject])
    finally:
        call(host, fingerprint, "session.logout", [session])


def main(arguments):
    if len(arguments) != 3:
        print("usage: force_logout.py HOST SHA256_FINGERPRINT SUBJECT_IDENTIFIER", file=sys.stderr)
        return 2
    host, fingerprint, subject = arguments
    path = os.environ.get("XENADMIN_LAB_PASSWORD_FILE")
    if not path:
        print("XENADMIN_LAB_PASSWORD_FILE must name the lab-only root password file", file=sys.stderr)
        return 2
    with open(path, encoding="utf-8") as file:
        password = file.readline().strip()
    try:
        force_logout(host, fingerprint, subject, password)
    except (Refused, ValueError, OSError) as error:
        print(f"force_logout: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"logged_out": subject}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
