#!/usr/bin/env python3
"""Exercise the packaged Flutter runner with fake SDK commands; never boot real devices."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BINARY = Path(os.environ.get('DEVSTART_TEST_BINARY', 'target/release/dev-start')).resolve()


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='devstart flutter ')
        self.root = Path(self.tmp.name)
        self.env = dict(os.environ, DEV_START_DATA_DIR=str(self.root / 'data'), FIXTURE_ROOT=str(self.root))
        self.env.pop('FLUTTER_OHOS_ROOT', None)
        self.env['PATH'] = str(self.root) + os.pathsep + self.env.get('PATH', '')
        self.flutter = self.tool('flutter', '''
root=Path(os.environ['FIXTURE_ROOT'])
args=sys.argv[1:]
if args==['build','--help']: print('hap'); sys.exit(0)
if args==['devices','--machine']:
    config=json.loads((root/'devices.json').read_text())
    print(json.dumps(config if not (root/'need-boot').exists() or (root/'booted').exists() else []));sys.exit(0)
if args[0]=='run':
    (root/'run.json').write_text(json.dumps(args));sys.exit(0)
sys.exit(3)
''')

    def tearDown(self):
        self.tmp.cleanup()

    def tool(self, name, body):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#!' + sys.executable + '\nimport os,sys,json\nfrom pathlib import Path\n' + body)
        path.chmod(0o755)
        return path

    def devices(self, platform='android-arm64', emulator=True, count=1):
        values = [dict(id=f'device-{i}', name=f'Demo {i}', targetPlatform=platform, emulator=emulator, isSupported=True) for i in range(count)]
        (self.root/'devices.json').write_text(json.dumps(values))

    def run_launch(self, platform='Android', target='Simulator', **extra):
        config = dict(program=str(self.flutter), platform=platform, target=target, **extra)
        return subprocess.run([str(BINARY), '--flutter-launch', json.dumps(config), '--flavor', 'demo'], env=self.env, cwd=self.root, capture_output=True, text=True, timeout=30)

    def test_connected_device_and_literal_arguments(self):
        self.devices(emulator=False)
        result=self.run_launch(target='Device',entrypoint='lib/main demo.dart')
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(json.loads((self.root/'run.json').read_text()), ['run','--debug','-d','device-0','-t','lib/main demo.dart','--flavor','demo'])

    def test_ambiguous_devices_require_selection(self):
        self.devices(emulator=False,count=2)
        result=self.run_launch(target='Device')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('多个可用设备',result.stderr)
        self.assertEqual(self.run_launch(target='Device',device='device-1').returncode,0)

    def test_missing_device_does_not_start_simulator_for_physical_target(self):
        (self.root/'devices.json').write_text('[]')
        result=self.run_launch(target='Device')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('未发现可用',result.stderr)
        self.assertFalse((self.root/'run.json').exists())

    def test_android_boot_before_debug(self):
        self.devices()
        (self.root/'need-boot').touch()
        self.tool('sdk/emulator/emulator', '''
if sys.argv[1:]==['-list-avds']: print('Demo_Android'); sys.exit(0)
assert sys.argv[1:]==['-avd','Demo_Android']
Path(os.environ['FIXTURE_ROOT'],'booted').touch()
import time
end=time.monotonic()+10
while not Path(os.environ['FIXTURE_ROOT'],'run.json').exists() and time.monotonic()<end: time.sleep(0.01)
time.sleep(0.1)
print('emulator alive after debugger exit',flush=True)
Path(os.environ['FIXTURE_ROOT'],'emulator-alive').touch()
''')
        self.env['ANDROID_HOME']=str(self.root/'sdk')
        result=self.run_launch()
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue((self.root/'booted').exists())
        import time
        end=time.monotonic()+3
        while not (self.root/'emulator-alive').exists() and time.monotonic()<end:
            time.sleep(0.03)
        self.assertTrue((self.root/'emulator-alive').exists(), 'SDK service must survive debug launcher replacement')

    def test_emulator_failure_keeps_actual_error_log(self):
        self.devices('ohos-arm64')
        (self.root/'need-boot').touch()
        emulator=self.tool('broken-emulator', "if sys.argv[1:]==['-list','-details']: print(json.dumps([{'name':'Broken'}]));sys.exit(0)\nprint('fixture boot failure',file=sys.stderr)\nsys.exit(9)\n")
        self.env['DEVECO_EMULATOR']=str(emulator)
        result=self.run_launch(platform='Ohos')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('模拟器启动失败',result.stderr)
        logs=list((self.root/'data/emulator-logs').glob('run-*.log'))
        self.assertTrue(logs)
        self.assertIn('fixture boot failure',logs[0].read_text())
        self.assertFalse((self.root/'run.json').exists())

    def test_unsupported_ohos_sdk_fails_before_boot(self):
        self.devices('ohos-arm64')
        self.flutter=self.tool('official-flutter', "print('apk ios web')\n")
        result=self.run_launch(platform='Ohos')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('不支持鸿蒙',result.stderr)
        self.assertFalse((self.root/'run.json').exists())

    def test_ohos_boot_before_debug(self):
        self.devices('ohos-arm64')
        (self.root/'need-boot').touch()
        emulator=self.tool('Emulator', '''
if sys.argv[1:]==['-list','-details']: print(json.dumps([{'name':'Demo Phone'}]));sys.exit(0)
assert sys.argv[1:]==['-start','Demo Phone']
Path(os.environ['FIXTURE_ROOT'],'booted').touch()
''')
        self.env['DEVECO_EMULATOR']=str(emulator)
        result=self.run_launch(platform='Ohos')
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue((self.root/'booted').exists())

    @unittest.skipUnless(sys.platform=='darwin','iOS host check is intentionally enforced')
    def test_ios_boot_before_debug(self):
        self.devices('ios')
        (self.root/'need-boot').touch()
        self.tool('xcrun', '''
if sys.argv[1:]==['simctl','list','devices','available','--json']: print(json.dumps({'devices':{'com.apple.CoreSimulator.SimRuntime.iOS-18':[{'udid':'demo-ios'}]}}));sys.exit(0)
assert sys.argv[1:]==['simctl','boot','demo-ios']
Path(os.environ['FIXTURE_ROOT'],'booted').touch()
''')
        self.tool('open','assert sys.argv[1:]==["-a","Simulator"]\n')
        result=self.run_launch(platform='Ios')
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue((self.root/'booted').exists())


if __name__=='__main__':
    unittest.main()
