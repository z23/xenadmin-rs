import json
from pathlib import Path
import tempfile
import unittest
from artifacts import stage


class CargoArtifacts(unittest.TestCase):
    def test_target_specific_reported_paths_win_over_stale_default_release(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            messages = []
            for name in ('xenadmin-rs', 'host-probe'):
                stale = root / 'release' / name
                stale.parent.mkdir(exist_ok=True)
                stale.write_bytes(b'stale executable')
                built = root / 'x86_64-unknown-linux-gnu/release' / name
                built.parent.mkdir(parents=True, exist_ok=True)
                built.write_bytes(b'\x7fELF\x02\x01' + bytes(12) + (62).to_bytes(2, 'little') + b'new build')
                messages.append({'reason':'compiler-artifact','target':{'name':name,'kind':['bin']},'profile':{'test':False},'executable':str(built)})
            log = root / 'build.jsonl'
            log.write_text('\n'.join(json.dumps(v) for v in messages))
            stage(log, root / 'package', 'x86_64')
            self.assertTrue((root/'package/xenadmin-rs').read_bytes().endswith(b'new build'))
            with self.assertRaises(ValueError):
                stage(log, root/'wrong', 'aarch64')
            self.assertFalse((root/'wrong').exists())
            log.write_text(json.dumps(messages[0]))
            with self.assertRaises(ValueError):
                stage(log, root/'partial', 'x86_64')
            self.assertFalse((root/'partial').exists())
