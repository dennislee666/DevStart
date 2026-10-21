#!/usr/bin/env python3
"""Offline checks for release credential validation; no keychain or remote mutations."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('signing', Path(__file__).with_name('sign-notarize-macos.py'))
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class SigningConfiguration(unittest.TestCase):
    def base(self):
        return {'MACOS_CERTIFICATE_P12_BASE64': 'dGVzdA==', 'MACOS_CERTIFICATE_PASSWORD': 'test', 'MACOS_SIGNING_IDENTITY': 'Developer ID Application: Example (ABCDEFGHIJ)', 'APPLE_TEAM_ID': 'ABCDEFGHIJ'}

    def test_missing_secrets_fail_closed(self):
        with self.assertRaisesRegex(ValueError, 'Missing signing'):
            signing.configuration({})

    def test_partial_api_key_does_not_silently_use_password(self):
        env = dict(self.base(), APPLE_API_KEY_ID='key', APPLE_ID='test@example.invalid', APPLE_APP_SPECIFIC_PASSWORD='test')
        with self.assertRaisesRegex(ValueError, 'APPLE_API_KEY_P8_BASE64'):
            signing.configuration(env)

    def test_authentication_modes_and_distribution_identity(self):
        env = dict(self.base(), APPLE_ID='test@example.invalid', APPLE_APP_SPECIFIC_PASSWORD='test')
        self.assertEqual(signing.configuration(env), 'apple_id')
        env['MACOS_SIGNING_IDENTITY'] = 'Apple Distribution: Example (ABCDEFGHIJ)'
        with self.assertRaisesRegex(ValueError, 'Developer ID Application'):
            signing.configuration(env)
        env = dict(self.base(), APPLE_API_KEY_P8_BASE64='dGVzdA==', APPLE_API_KEY_ID='key')
        self.assertEqual(signing.configuration(env), 'api_key')

    def test_private_material_uses_restricted_permissions(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'key'
            signing.private_file(path, b'test')
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                signing.private_file(path, b'replacement')


if __name__ == '__main__':
    unittest.main()
