"""CI portable archive check, Python stdlib only; never runs the packed EXE."""
import io
import hashlib
import importlib.util
import json
import posixpath
import struct
import sys
import tempfile
import zipfile
from pathlib import Path


def check_portable(file, expected=None):
    with zipfile.ZipFile(file) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError('duplicate ZIP entry')
        prefix = '' if 'bull-arrives.exe' in names else 'BullArrives/'
        required = {prefix + name for name in ('bull-arrives.exe', 'portable.dat', 'README.md', 'RELEASE-GUIDE.md')}
        if not required.issubset(names):
            raise ValueError('portable EXE/marker/documentation missing')
        runtime_prefix = prefix + 'research-runtime/'
        marker = runtime_prefix + 'runtime.json'
        if marker not in names:
            raise ValueError('portable private research runtime missing')
        manifest = json.loads(archive.read(marker))
        if manifest.get('schema') != 'bull-research-runtime-v1':
            raise ValueError('invalid research runtime schema')
        if expected is None:
            spec = importlib.util.spec_from_file_location('research_bundle', Path(__file__).with_name('research-bundle.py'))
            bundle = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(bundle)
            expected = bundle.expected_files(bundle.ROOT)
        if manifest['files'] != expected:
            raise ValueError('portable frozen manifest differs from application source')
        for name, sha in expected.items():
            with archive.open(runtime_prefix + name) as source:
                if hashlib.file_digest(source, 'sha256').hexdigest() != sha:
                    raise ValueError('portable frozen fingerprint mismatch: ' + name)
        if runtime_prefix + 'python-x86_64/python.exe' not in names:
            raise ValueError('portable Python missing')
        for item in archive.infolist():
            if item.is_dir() and item.filename in (prefix, runtime_prefix):
                continue
            resource = item.filename.startswith(runtime_prefix)
            relative = item.filename[len(runtime_prefix):] if resource else item.filename
            unsafe = '\\' in relative or ':' in relative or relative.startswith('/') or posixpath.normpath(relative) != relative.rstrip('/') or '..' in relative.split('/')
            known_directory = item.is_dir() and any(name.startswith(relative) for name in expected)
            if (item.filename not in required and not resource) or unsafe or item.flag_bits & 1 or (item.external_attr >> 16) & 0o170000 == 0o120000 or (resource and not known_directory and relative not in ('runtime.json', *expected) and not relative.startswith('python-x86_64/')):
                raise ValueError('unexpected/private/unsafe ZIP entry: ' + item.filename)
        if archive.testzip() is not None:
            raise ValueError('ZIP CRC failed')
        exe = archive.read(prefix + 'bull-arrives.exe')
        if len(exe) < 64 or exe[:2] != b'MZ':
            raise ValueError('portable program is not an EXE')
        offset = struct.unpack_from('<I', exe, 0x3c)[0]
        if offset + 6 > len(exe) or exe[offset:offset + 4] != b'PE\0\0' or struct.unpack_from('<H', exe, offset + 4)[0] != 0x8664:
            raise ValueError('portable program is not Windows x64 PE')
        return len(exe)


def demo():
    exe = bytearray(128)
    exe[:2] = b'MZ'
    struct.pack_into('<I', exe, 0x3c, 64)
    exe[64:68] = b'PE\0\0'
    struct.pack_into('<H', exe, 68, 0x8664)
    expected = {'research/test.json': hashlib.sha256(b'fixture').hexdigest()}
    def sample(extra=None):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            for name, data in [('bull-arrives.exe', exe), ('portable.dat', b''), ('README.md', b'readme'), ('RELEASE-GUIDE.md', b'guide')]:
                archive.writestr(name, data)
            archive.writestr('research-runtime/runtime.json', json.dumps({'schema': 'bull-research-runtime-v1', 'files': expected}))
            archive.writestr('research-runtime/research/test.json', b'fixture')
            archive.writestr('research-runtime/python-x86_64/python.exe', exe)
            if extra:
                archive.writestr(extra, b'private')
        buffer.seek(0)
        return buffer
    assert check_portable(sample(), expected) == 128
    for extra in ('data/bull-arrives.db', '../secret.key', 'research-runtime/../secret', 'research-runtime/research/private.db'):
        try:
            check_portable(sample(extra), expected)
        except ValueError:
            continue
        raise AssertionError('unsafe archive accepted')
    print('portable ZIP self-check passed: required files, x64 PE, CRC, no user data/path traversal')


if __name__ == '__main__':
    if sys.argv[1:] == ['--self-test']:
        demo()
    elif len(sys.argv) == 2:
        print('portable ZIP passed: Windows x64 EXE bytes=' + str(check_portable(sys.argv[1])))
    else:
        raise SystemExit('Usage: python check-portable.py <portable.zip> | --self-test')
