import hashlib
import json
import unittest
from unittest import mock

import other_admin


class OtherAdminTests(unittest.TestCase):
    def test_a_fingerprint_is_compared_without_its_separators_or_case(self):
        certificate = b"synthetic certificate"
        digest = hashlib.sha256(certificate).hexdigest()
        spaced = ":".join(digest[i:i + 2] for i in range(0, 64, 2)).upper()
        self.assertTrue(other_admin.matches(certificate, digest))
        self.assertTrue(other_admin.matches(certificate, spaced))
        self.assertFalse(other_admin.matches(b"another certificate", digest))
        with self.assertRaises(ValueError):
            other_admin.normalized(digest[:-2])
        with self.assertRaises(ValueError):
            other_admin.normalized("z" * 64)

    def test_an_error_reply_is_reported_by_its_code_alone(self):
        reply = json.dumps({"jsonrpc": "2.0", "id": 1, "error": {
            "code": 1, "message": "SESSION_AUTHENTICATION_FAILED", "data": ["root", "synthetic-secret"]}})
        with self.assertRaises(other_admin.Refused) as refused:
            other_admin.result(reply)
        self.assertIn("SESSION_AUTHENTICATION_FAILED", str(refused.exception))
        self.assertNotIn("synthetic-secret", str(refused.exception))
        self.assertEqual(other_admin.result('{"jsonrpc":"2.0","id":1,"result":"OpaqueRef:x"}'), "OpaqueRef:x")

    def run_action(self, name, arguments, answers=None, refuse=None):
        calls = []

        def call(host, fingerprint, method, params, timeout=30):
            calls.append((method, params))
            if method == refuse:
                raise other_admin.Refused(f"the host refused the call: RBAC_PERMISSION_DENIED")
            if method == "session.login_with_password":
                return "OpaqueRef:tooling"
            return (answers or {}).get(method, "")

        with mock.patch.object(other_admin, "call", call):
            try:
                outcome = other_admin.act("192.0.2.1", "ab" * 32, "synthetic-secret",
                                          other_admin.ACTIONS[name][0], arguments)
            except other_admin.Refused as error:
                outcome = error
        return calls, outcome

    def test_a_subject_is_logged_out_between_a_login_and_a_logout_of_the_tooling_session(self):
        calls, outcome = self.run_action("logout-subject", ["S-1-5-21-1"])
        self.assertEqual([c[0] for c in calls], [
            "session.login_with_password", "session.logout_subject_identifier", "session.logout"])
        self.assertEqual(calls[1][1], ["OpaqueRef:tooling", "S-1-5-21-1"])
        self.assertEqual(calls[2][1], ["OpaqueRef:tooling"])
        self.assertEqual(outcome, {"logged_out": "S-1-5-21-1"})

    def test_a_toolstack_restart_names_the_host_by_its_uuid(self):
        calls, outcome = self.run_action(
            "restart-toolstack", ["22222222-2222-2222-2222-222222222222"],
            answers={"host.get_by_uuid": "OpaqueRef:member"})
        self.assertEqual([c[0] for c in calls], [
            "session.login_with_password", "host.get_by_uuid", "host.restart_agent", "session.logout"])
        self.assertEqual(calls[1][1], ["OpaqueRef:tooling", "22222222-2222-2222-2222-222222222222"])
        self.assertEqual(calls[2][1], ["OpaqueRef:tooling", "OpaqueRef:member"])
        self.assertEqual(outcome, {"restarting": "22222222-2222-2222-2222-222222222222"})

    def test_pending_tasks_are_listed_by_name_and_nothing_is_written(self):
        tasks = {"OpaqueRef:a": {"name_label": "Async.VM.start", "status": "success"},
                 "OpaqueRef:b": {"name_label": "Connection to VM console", "status": "pending"}}
        calls, outcome = self.run_action("pending-tasks", [], answers={"task.get_all_records": tasks})
        self.assertEqual([c[0] for c in calls], [
            "session.login_with_password", "task.get_all_records", "session.logout"])
        self.assertEqual(outcome, {"pending": ["Connection to VM console"]})

    def test_the_tooling_session_is_closed_when_the_host_refuses_the_action(self):
        calls, outcome = self.run_action("logout-subject", ["S-1-5-21-1"],
                                         refuse="session.logout_subject_identifier")
        self.assertIsInstance(outcome, other_admin.Refused)
        self.assertEqual(calls[-1][0], "session.logout")

    def test_the_password_is_not_taken_from_the_command_line(self):
        arguments = ["192.0.2.1", "ab" * 32, "logout-subject", "S-1-5-21-1"]
        with mock.patch.dict("os.environ", {}, clear=True):
            self.assertEqual(other_admin.main(arguments), 2)
        self.assertEqual(other_admin.main(arguments + ["extra"]), 2)
        self.assertEqual(other_admin.main(["192.0.2.1", "ab" * 32, "delete-everything"]), 2)
        self.assertEqual(other_admin.main(["192.0.2.1", "ab" * 32, "pending-tasks", "extra"]), 2)


if __name__ == "__main__":
    unittest.main()
