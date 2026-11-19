from pathlib import Path
import tempfile
import unittest
import smoke

ROOT = Path(__file__).resolve().parent.parent


def script(directory, body):
    path = Path(directory) / 'client'
    path.write_text('#!/bin/sh\n' + body)
    path.chmod(0o755)
    return path


class Smoke(unittest.TestCase):
    def test_the_refusal_is_the_one_the_client_prints(self):
        source = (ROOT / 'crates/xenadmin-desktop/src/app.rs').read_text()
        self.assertIn(smoke.REFUSAL, source)

    def test_a_client_that_refuses_as_packaged_passes(self):
        with tempfile.TemporaryDirectory() as temp:
            client = script(temp, f"echo 'xenadmin-rs: {smoke.REFUSAL}.' >&2\nexit 1\n")
            self.assertIsNone(smoke.check(client))

    def test_the_scene_is_asked_for_and_no_display_is_named(self):
        with tempfile.TemporaryDirectory() as temp:
            client = script(temp, '[ "$1 $2 $3" = "--scene demo --snap" ] || exit 9\n'
                                  '[ -z "$DISPLAY$WAYLAND_DISPLAY$WAYLAND_SOCKET" ] || exit 8\n'
                                  '[ -d "$XENADMIN_STATE_DIR" ] || exit 7\n'
                                  f"echo '{smoke.REFUSAL}' >&2\nexit 1\n")
            self.assertIsNone(smoke.check(client))
            self.assertEqual(smoke.main([str(client)]), 0)

    def test_an_appimage_is_run_extracted(self):
        with tempfile.TemporaryDirectory() as temp:
            client = script(temp, '[ "$APPIMAGE_EXTRACT_AND_RUN" = 1 ] || exit 9\n'
                                  f"echo '{smoke.REFUSAL}' >&2\nexit 1\n")
            self.assertIsNone(smoke.check(client, appimage=True))
            self.assertIn('status 9', smoke.check(client))

    def test_a_client_that_does_not_load_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            client = script(temp, 'echo "error while loading shared libraries: libxcb.so.1" >&2\nexit 127\n')
            wrong = smoke.check(client)
            self.assertIn('status 127', wrong)
            self.assertIn('libxcb.so.1', wrong)
            self.assertEqual(smoke.main([str(client)]), 1)

    def test_any_other_answer_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertIn('status 0', smoke.check(script(temp, 'exit 0\n')))
            self.assertIn('nothing', smoke.check(script(temp, 'exit 1\n')))
            self.assertIn('status 101', smoke.check(script(temp, f"echo '{smoke.REFUSAL}' >&2\nexit 101\n")))
            self.assertIn('could not be started', smoke.check(Path(temp) / 'absent'))

    def test_a_client_that_wrote_a_file_fails(self):
        with tempfile.TemporaryDirectory() as temp:
            client = script(temp, f"touch \"$4\"\necho '{smoke.REFUSAL}' >&2\nexit 1\n")
            self.assertIn('wrote a file', smoke.check(client))
            client = script(temp, f"touch \"$XENADMIN_STATE_DIR/desktop.lock\"\necho '{smoke.REFUSAL}' >&2\nexit 1\n")
            self.assertIn('wrote a file', smoke.check(client))

    def test_one_executable_is_named(self):
        self.assertEqual(smoke.main([]), 2)
        self.assertEqual(smoke.main(['a', 'b']), 2)


if __name__ == '__main__':
    unittest.main()
