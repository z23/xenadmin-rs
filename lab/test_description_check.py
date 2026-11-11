import unittest

import description_check
import other_admin

DESCRIPTION = [
    {"name": "VM", "fields": [{"name": "uuid"}, {"name": "power_state"}, {"name": "parent"}],
     "messages": [{"name": "get_all_records"}, {"name": "start"}]},
    {"name": "host", "fields": [{"name": "uuid"}], "messages": [{"name": "get_all_records"}]},
    {"name": "VTPM", "fields": [{"name": "uuid"}], "messages": [{"name": "get_all_records"}]},
    {"name": "VBD_metrics", "fields": [{"name": "uuid"}], "messages": [{"name": "get_all_records"}]},
    {"name": "session", "fields": [{"name": "uuid"}], "messages": [{"name": "logout"}]},
]


class DescriptionCheckTests(unittest.TestCase):
    def compare(self, answers):
        asked = []

        def ask(method, *params):
            asked.append(method)
            if method == "VBD_metrics.get_all_records":
                raise other_admin.Refused("the host refused the call: MESSAGE_REMOVED")
            return answers.get(method, {})

        return description_check.compare(DESCRIPTION, ask), asked

    def test_fields_are_compared_in_both_directions_by_name_alone(self):
        outcome, asked = self.compare({
            "VM.get_all_records": {
                "OpaqueRef:a": {"uuid": "synthetic-a", "power_state": "Running", "groups": []},
                "OpaqueRef:b": {"uuid": "synthetic-b", "power_state": "Halted"}},
            "host.get_all_records": {"OpaqueRef:h": {"uuid": "synthetic-h"}},
        })
        self.assertEqual(outcome["compared"], ["VM", "host"])
        self.assertEqual(outcome["live_fields_the_description_lacks"], {"VM": ["groups"]})
        self.assertEqual(outcome["described_fields_no_record_carries"], {"VM": ["parent"]})
        # No value of any record is in the outcome, only names.
        self.assertNotIn("synthetic", str(outcome))
        # Only reads are asked for, and a class without the call is not asked at all.
        self.assertTrue(all(method.endswith(".get_all_records") for method in asked))
        self.assertNotIn("session.get_all_records", asked)

    def test_a_class_without_records_or_refused_by_the_host_is_named_not_compared(self):
        outcome, _ = self.compare({"host.get_all_records": {"OpaqueRef:h": {"uuid": "synthetic-h"}}})
        self.assertEqual(outcome["compared"], ["host"])
        self.assertEqual(outcome["without_records"], ["VM", "VTPM"])
        self.assertEqual(list(outcome["refused"]), ["VBD_metrics"])
        self.assertIn("MESSAGE_REMOVED", outcome["refused"]["VBD_metrics"])
        self.assertEqual(outcome["live_fields_the_description_lacks"], {})

    def test_wrong_arguments_print_the_usage_and_send_nothing(self):
        self.assertEqual(description_check.main([]), 2)
        self.assertEqual(description_check.main(["host"]), 2)


if __name__ == "__main__":
    unittest.main()
