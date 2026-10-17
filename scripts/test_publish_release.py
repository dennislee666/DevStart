"""Offline checks for destructive release-selection boundaries and package integrity."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('publisher', Path(__file__).with_name('publish-release.py'))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class ReleaseChecks(unittest.TestCase):
    def test_prune_selects_only_intermediate_builds_of_exact_version(self):
        self.assertTrue(publisher.superseded('v1.0.0-build.123', '1.0.0'))
        for tag in ['v1.0.0', 'v1.0.01-build.1', 'v1.1.0-build.1', 'personal']:
            self.assertFalse(publisher.superseded(tag, '1.0.0'))

    def test_package_tampering_and_missing_artifacts_are_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            files = [f'DevStart-1.0.0-macOS-universal.{ext}' for ext in ['dmg', 'zip']]
            for file in files:
                (root / file).write_bytes(b'fixture')
            (root / 'SHA256SUMS.txt').write_text(''.join(hashlib.sha256(b'fixture').hexdigest() + '  ' + file + '\n' for file in files))
            self.assertEqual(len(publisher.packages(root, '1.0.0')), 3)
            (root / files[0]).write_bytes(b'tampered')
            with self.assertRaises(ValueError):
                publisher.packages(root, '1.0.0')

    def test_manifest_cannot_reference_files_outside_artifact_directory(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'SHA256SUMS.txt').write_text('0' * 64 + '  ../outside.zip\n')
            with self.assertRaises(ValueError):
                publisher.packages(root, '1.0.0')


if __name__ == '__main__':
    unittest.main()
