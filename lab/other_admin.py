#!/usr/bin/env python3
"""What another administrator does to a lab pool while the client under test is connected.

The live harness uses this for actions the client cannot take itself in the situation
being tested, so that it can watch how the client copes:

    other_admin.py HOST SHA256_FINGERPRINT logout-subject SUBJECT_IDENTIFIER
        Close every session of one directory subject. XenCenter offers this as "Logout
        User"; the client has no operation for it.
    other_admin.py HOST SHA256_FINGERPRINT restart-toolstack HOST_UUID
        Restart the toolstack of one pool host. The client refuses that while a console is
        open, which is exactly when the harness needs it.
    other_admin.py HOST SHA256_FINGERPRINT pending-tasks
        Print the names of the pool's unfinished tasks. Read-only.

HOST is the pool coordinator. The script logs in as the lab's root account, makes the one
call and logs out. The password is read from the file named by XENADMIN_LAB_PASSWORD_FILE
and is never taken from the command line or printed. Nothing is sent to a host whose
certificate does not match the fingerprint. XAPI never closes a local superuser's session
by a subject logout, so the account used here keeps its own.
"""
import hashlib
import http.client
import json
import os
import ssl
import sys

USAGE = "usage: other_admin.py HOST SHA256_FINGERPRINT logout-subject SUBJECT | restart-toolstack HOST_UUID | pending-tasks"


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


def logout_subject(ask, subject):
    ask("session.logout_subject_identifier", subject)
    return {"logged_out": subject}


def restart_toolstack(ask, host_uuid):
    ask("host.restart_agent", ask("host.get_by_uuid", host_uuid))
    return {"restarting": host_uuid}


def pending_tasks(ask):
    tasks = ask("task.get_all_records")
    return {"pending": sorted(t["name_label"] for t in tasks.values() if t["status"] == "pending")}


ACTIONS = {"logout-subject": (logout_subject, 1), "restart-toolstack": (restart_toolstack, 1),
           "pending-tasks": (pending_tasks, 0)}


def act(host, fingerprint, password, action, arguments):
    """Runs one action between a login and a logout of the tooling's own session."""
    session = call(host, fingerprint, "session.login_with_password",
                   ["root", password, "1.0", "xenadmin-rs-lab-tooling"])
    try:
        return action(lambda method, *params: call(host, fingerprint, method, [session, *params]),
                      *arguments)
    finally:
        call(host, fingerprint, "session.logout", [session])


def main(arguments):
    if len(arguments) < 3 or arguments[2] not in ACTIONS or len(arguments) - 3 != ACTIONS[arguments[2]][1]:
        print(USAGE, file=sys.stderr)
        return 2
    host, fingerprint, name = arguments[:3]
    path = os.environ.get("XENADMIN_LAB_PASSWORD_FILE")
    if not path:
        print("XENADMIN_LAB_PASSWORD_FILE must name the lab-only root password file", file=sys.stderr)
        return 2
    with open(path, encoding="utf-8") as file:
        password = file.readline().strip()
    try:
        outcome = act(host, fingerprint, password, ACTIONS[name][0], arguments[3:])
    except (Refused, ValueError, OSError) as error:
        print(f"other_admin: {error}", file=sys.stderr)
        return 1
    print(json.dumps(outcome))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
