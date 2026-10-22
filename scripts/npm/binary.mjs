import assert from 'node:assert/strict';

export function checkBinary(key, bytes) {
  assert.ok(bytes.length >= 64, 'truncated binary');
  if (key === 'win32-x64') {
    assert.equal(bytes.toString('ascii', 0, 2), 'MZ');
    const pe = bytes.readUInt32LE(60);
    assert.ok(pe + 6 <= bytes.length);
    assert.equal(bytes.toString('ascii', pe, pe + 4), 'PE\0\0');
    assert.equal(bytes.readUInt16LE(pe + 4), 0x8664, 'PE must be x64');
  } else if (key.startsWith('linux-')) {
    assert.deepEqual([...bytes.subarray(0, 6)], [127, 69, 76, 70, 2, 1], 'ELF64 little endian required');
    assert.equal(bytes.readUInt16LE(18), key.endsWith('arm64') ? 183 : 62);
    const offset = Number(bytes.readBigUInt64LE(32));
    const size = bytes.readUInt16LE(54);
    const count = bytes.readUInt16LE(56);
    assert.ok(size >= 56 && count > 0 && offset + size * count <= bytes.length);
    for (let i = 0; i < count; i++) assert.notEqual(bytes.readUInt32LE(offset + i * size), 3, 'Linux release requires static musl: PT_INTERP found');
  } else {
    assert.ok(key === 'darwin-x64' || key === 'darwin-arm64');
    assert.equal(bytes.readUInt32LE(0), 0xfeedfacf, 'Mach-O64 required');
    assert.equal(bytes.readUInt32LE(4), key.endsWith('arm64') ? 0x0100000c : 0x01000007);
  }
}
