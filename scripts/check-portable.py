"""CI portable archive check, Python stdlib only; never runs the packed EXE."""
import io
import struct
import sys
import tempfile
import zipfile
from pathlib import Path


def check_portable(file):
    with zipfile.ZipFile(file) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError('duplicate ZIP entry')
        prefix = '' if 'bull-arrives.exe' in names else 'BullArrives/'
        required = {prefix + name for name in ('bull-arrives.exe', 'portable.dat', 'README.md', 'RELEASE-GUIDE.md')}
        if not required.issubset(names):
            raise ValueError('portable EXE/marker/documentation missing')
        for item in archive.infolist():
            if item.is_dir() and item.filename == prefix:
                continue
            if item.filename not in required or item.flag_bits & 1 or (item.external_attr >> 16) & 0o170000 == 0o120000:
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
    def sample(extra=None):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            for name, data in [('bull-arrives.exe', exe), ('portable.dat', b''), ('README.md', b'readme'), ('RELEASE-GUIDE.md', b'guide')]:
                archive.writestr(name, data)
            if extra:
                archive.writestr(extra, b'private')
        buffer.seek(0)
        return buffer
    assert check_portable(sample()) == 128
    for extra in ('data/bull-arrives.db', '../secret.key'):
        try:
            check_portable(sample(extra))
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
