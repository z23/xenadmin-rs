import hashlib
import json
import unittest
from unittest import mock

import force_logout


class ForceLogoutTests(unittest.TestCase):
    def test_a_fingerprint_is_compared_without_its_separators_or_case(self):
        certificate = b"synthetic certificate"
        digest = hashlib.sha256(certificate).hexdigest()
        spaced = ":".join(digest[i:i + 2] for i in range(0, 64, 2)).upper()
        self.assertTrue(force_logout.matches(certificate, digest))
        self.assertTrue(force_logout.matches(certificate, spaced))
        self.assertFalse(force_logout.matches(b"another certificate", digest))
        with self.assertRaises(ValueError):
            force_logout.normalized(digest[:-2])
        with self.assertRaises(ValueError):
            force_logout.normalized("z" * 64)

    def test_an_error_reply_is_reported_by_its_code_alone(self):
        reply = json.dumps({"jsonrpc": "2.0", "id": 1, "error": {
            "code": 1, "message": "SESSION_AUTHENTICATION_FAILED", "data": ["root", "synthetic-secret"]}})
        with self.assertRaises(force_logout.Refused) as refused:
            force_logout.result(reply)
        self.assertIn("SESSION_AUTHENTICATION_FAILED", str(refused.exception))
        self.assertNotIn("synthetic-secret", str(refused.exception))
        self.assertEqual(force_logout.result('{"jsonrpc":"2.0","id":1,"result":"OpaqueRef:x"}'), "OpaqueRef:x")

    def test_the_subject_is_logged_out_between_a_login_and_a_logout_of_the_tooling_session(self):
        calls = []

        def call(host, fingerprint, method, params, timeout=30):
            calls.append((host, fingerprint, method, params))
            return "OpaqueRef:tooling" if method == "session.login_with_password" else ""

        with mock.patch.object(force_logout, "call", call):
            force_logout.force_logout("192.0.2.1", "ab" * 32, "S-1-5-21-1", "synthetic-secret")
        self.assertEqual([c[2] for c in calls], [
            "session.login_with_password", "session.logout_subject_identifier", "session.logout"])
        self.assertEqual(calls[1][3], ["OpaqueRef:tooling", "S-1-5-21-1"])
        self.assertEqual(calls[2][3], ["OpaqueRef:tooling"])

    def test_the_tooling_session_is_closed_when_the_host_refuses_the_logout(self):
        calls = []

        def call(host, fingerprint, method, params, timeout=30):
            calls.append(method)
            if method == "session.logout_subject_identifier":
                raise force_logout.Refused("the host refused the call: RBAC_PERMISSION_DENIED")
            return "OpaqueRef:tooling"

        with mock.patch.object(force_logout, "call", call):
            with self.assertRaises(force_logout.Refused):
                force_logout.force_logout("192.0.2.1", "ab" * 32, "S-1-5-21-1", "synthetic-secret")
        self.assertEqual(calls[-1], "session.logout")

    def test_the_password_is_not_taken_from_the_command_line(self):
        with mock.patch.dict("os.environ", {}, clear=True):
            self.assertEqual(force_logout.main(["192.0.2.1", "ab" * 32, "S-1-5-21-1"]), 2)
        self.assertEqual(force_logout.main(["192.0.2.1", "ab" * 32, "S-1-5-21-1", "extra"]), 2)


if __name__ == "__main__":
    unittest.main()
