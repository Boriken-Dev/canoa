import 'dart:typed_data';
import 'dart:js_interop';

import 'package:web/web.dart' as web;

/// Reads [blob], patches the WebM Duration field to [durationMs], and returns
/// a new Blob. Returns [blob] unchanged if it already has a valid duration or
/// the structure cannot be parsed.
Future<web.Blob> fixWebmDuration(web.Blob blob, int durationMs) async {
  final jsBuffer = await blob.arrayBuffer().toDart;
  final src = jsBuffer.toDart.asUint8List();

  final patched = _patchDuration(src, durationMs.toDouble());

  if (patched == null) return blob;

  return web.Blob([patched.toJS].toJS, web.BlobPropertyBag(type: blob.type));
}

// --- EBML patching -----------------------------------------------------------
// Spec: https://www.matroska.org/technical/elements.html

// Matroska/WebM element IDs (from the EBML spec).
const _kSegmentId = 0x8538067;      // top-level container
const _kInfoId = 0x549a966;         // segment metadata block
const _kTimecodeScaleId = 0xad7b1;  // nanoseconds per timecode unit (default 1 ms)
const _kDurationId = 0x489;         // playback duration in timecode units

/// Returns a patched [Uint8List] with Duration set, or null if no patch was
/// needed (already has valid duration) or the structure is unrecognisable.
Uint8List? _patchDuration(Uint8List data, double durationMs) {
  var pos = 0;
  while (pos < data.length) {
    final id = _readVint(data, pos);
    if (id == null) return null;
    pos += id.byteCount;

    final size = _readSize(data, pos);
    if (size == null) return null;
    pos += size.byteCount;

    if (id.value == _kSegmentId) {
      final segEnd = size.isUnknown
          ? data.length
          : (pos + size.value).clamp(0, data.length);
      return _patchSegment(data, pos, segEnd, durationMs);
    }

    if (size.isUnknown) return null;
    pos += size.value;
  }
  return null;
}

/// Scans the Segment body for the Info element, then delegates to [_patchInfoSection].
Uint8List? _patchSegment(
  Uint8List data,
  int start,
  int end,
  double durationMs,
) {
  var pos = start;
  while (pos < end) {
    final id = _readVint(data, pos);
    if (id == null) return null;
    pos += id.byteCount;

    final size = _readSize(data, pos);
    if (size == null) return null;
    final sizeVintPos = pos;
    final sizeVintLen = size.byteCount;
    pos += size.byteCount;

    if (id.value == _kInfoId) {
      final infoEnd = size.isUnknown ? end : (pos + size.value).clamp(0, end);
      return _patchInfoSection(
        data,
        pos,
        infoEnd,
        sizeVintPos,
        sizeVintLen,
        durationMs,
      );
    }

    if (size.isUnknown) return null;
    pos += size.value;
  }
  return null;
}

/// Patches the Info section: overwrites Duration in-place if present and zero,
/// or inserts a new Duration element at the end of Info if absent (Chromium recorders omit it).
Uint8List? _patchInfoSection(
  Uint8List data,
  int start,
  int end,
  int infoSizeVintPos,  // byte offset of the Info size VINT, needed when we grow Info
  int infoSizeVintLen,
  double durationMs,
) {
  var pos = start;
  int? durDataPos;
  int durDataSize = 0;
  int? tsPos;
  int tsSize = 0;

  while (pos < end) {
    final id = _readVint(data, pos);
    if (id == null) break;
    pos += id.byteCount;

    final size = _readSize(data, pos);
    if (size == null) break;
    pos += size.byteCount;

    if (id.value == _kDurationId) {
      durDataPos = pos;
      durDataSize = size.value;
    } else if (id.value == _kTimecodeScaleId) {
      tsPos = pos;
      tsSize = size.value;
    }

    if (size.isUnknown) break;
    pos += size.value;
  }

  // Duration element present — patch if zero, leave if already set.
  if (durDataPos != null) {
    if (durDataSize != 4 && durDataSize != 8) return null;

    final durBd = ByteData.sublistView(
      data,
      durDataPos,
      durDataPos + durDataSize,
    );
    final existing = durDataSize == 8
        ? durBd.getFloat64(0)
        : durBd.getFloat32(0);
    if (existing > 0) return null; // already valid

    final copy = Uint8List.fromList(data);
    final bd = ByteData.sublistView(copy, durDataPos, durDataPos + durDataSize);
    if (durDataSize == 8) {
      bd.setFloat64(0, durationMs);
    } else {
      bd.setFloat32(0, durationMs);
    }
    if (tsPos != null && tsSize >= 1 && tsSize <= 8) {
      _writeUintBE(
        ByteData.sublistView(copy, tsPos, tsPos + tsSize),
        1000000,
        tsSize,
      );
    }
    return copy;
  }

  // Duration element absent (Chromium based) — insert at end of Info.
  // Duration element: ID 0x4489 (2 bytes) + size 0x88 (1 byte) + float64 (8 bytes) = 11 bytes.
  final durElement = _buildDurationElement(durationMs);
  final oldInfoContentSize = end - start;
  final newInfoContentSize = oldInfoContentSize + durElement.length;
  final newSizeVint = _encodeSize(newInfoContentSize);
  if (newSizeVint == null) return null;

  final deltaVint = newSizeVint.length - infoSizeVintLen;
  final totalSize = data.length + durElement.length + deltaVint;
  final result = Uint8List(totalSize);
  var out = 0;

  // Bytes before the Info size VINT
  result.setRange(out, out + infoSizeVintPos, data);
  out += infoSizeVintPos;

  // New Info size VINT
  result.setRange(out, out + newSizeVint.length, newSizeVint);
  out += newSizeVint.length;

  // Original Info content (all existing elements)
  result.setRange(out, out + oldInfoContentSize, data, start);
  out += oldInfoContentSize;

  // Duration element appended at end of Info content
  result.setRange(out, out + durElement.length, durElement);
  out += durElement.length;

  // Everything after the Info section
  result.setRange(out, totalSize, data, end);

  // TimecodeScale is inside Info; adjust its position by deltaVint in result.
  if (tsPos != null && tsSize >= 1 && tsSize <= 8) {
    final newTsPos = tsPos + deltaVint;
    _writeUintBE(
      ByteData.sublistView(result, newTsPos, newTsPos + tsSize),
      1000000,
      tsSize,
    );
  }

  return result;
}

// Duration element: ID 0x4489, VINT size 0x88 (= 8 bytes), then 8-byte float64 BE.
Uint8List _buildDurationElement(double durationMs) {
  final el = Uint8List(11);
  el[0] = 0x44;
  el[1] = 0x89;
  el[2] = 0x88;
  ByteData.sublistView(el, 3).setFloat64(0, durationMs);
  return el;
}

/// Encodes [value] as a minimal EBML VINT size descriptor (1–4 bytes).
/// Returns null if [value] is too large for a 4-byte VINT.
Uint8List? _encodeSize(int value) {
  if (value < 0x7F) {
    return Uint8List.fromList([0x80 | value]);
  }
  if (value < 0x3FFF) {
    return Uint8List.fromList([0x40 | (value >> 8), value & 0xFF]);
  }
  if (value < 0x1FFFFF) {
    return Uint8List.fromList([
      0x20 | (value >> 16),
      (value >> 8) & 0xFF,
      value & 0xFF,
    ]);
  }
  if (value < 0x0FFFFFFF) {
    return Uint8List.fromList([
      0x10 | (value >> 24),
      (value >> 16) & 0xFF,
      (value >> 8) & 0xFF,
      value & 0xFF,
    ]);
  }
  return null;
}

/// Writes [value] as a big-endian unsigned integer into [bd].
void _writeUintBE(ByteData bd, int value, int byteCount) {
  for (var i = byteCount - 1; i >= 0; i--) {
    bd.setUint8(i, value & 0xFF);
    value >>= 8;
  }
}

// --- EBML VINT ---------------------------------------------------------------

class _Vint {
  final int value;
  final int byteCount;
  const _Vint(this.value, this.byteCount);
}

class _Size {
  final int value;
  final int byteCount;
  final bool isUnknown;
  const _Size(this.value, this.byteCount, {this.isUnknown = false});
}

/// Reads an EBML variable-length integer (element ID or numeric value).
/// The width is encoded in the leading byte: the first set bit marks the end of the width prefix.
_Vint? _readVint(Uint8List data, int offset) {
  if (offset >= data.length) return null;
  final first = data[offset];

  int width, mask;
  if (first & 0x80 != 0) {
    width = 1;
    mask = 0x7F;
  } else if (first & 0x40 != 0) {
    width = 2;
    mask = 0x3F;
  } else if (first & 0x20 != 0) {
    width = 3;
    mask = 0x1F;
  } else if (first & 0x10 != 0) {
    width = 4;
    mask = 0x0F;
  } else if (first & 0x08 != 0) {
    width = 5;
    mask = 0x07;
  } else if (first & 0x04 != 0) {
    width = 6;
    mask = 0x03;
  } else if (first & 0x02 != 0) {
    width = 7;
    mask = 0x01;
  } else if (first & 0x01 != 0) {
    width = 8;
    mask = 0x00;
  } else {
    return null;
  }

  if (offset + width > data.length) {
    return null;
  }

  var value = first & mask;
  for (var i = 1; i < width; i++) {
    value = (value << 8) | data[offset + i];
  }
  return _Vint(value, width);
}

/// Reads an EBML element size VINT. All-ones data bits signal an unknown/streaming size
/// (common for the top-level Segment in live recordings).
_Size? _readSize(Uint8List data, int offset) {
  if (offset >= data.length) return null;
  final first = data[offset];

  int width, mask;
  if (first & 0x80 != 0) {
    width = 1;
    mask = 0x7F;
  } else if (first & 0x40 != 0) {
    width = 2;
    mask = 0x3F;
  } else if (first & 0x20 != 0) {
    width = 3;
    mask = 0x1F;
  } else if (first & 0x10 != 0) {
    width = 4;
    mask = 0x0F;
  } else if (first & 0x08 != 0) {
    width = 5;
    mask = 0x07;
  } else if (first & 0x04 != 0) {
    width = 6;
    mask = 0x03;
  } else if (first & 0x02 != 0) {
    width = 7;
    mask = 0x01;
  } else if (first & 0x01 != 0) {
    width = 8;
    mask = 0x00;
  } else {
    return null;
  }

  if (offset + width > data.length) {
    return null;
  }

  var value = first & mask;
  var allOnes = value == mask;
  for (var i = 1; i < width; i++) {
    final b = data[offset + i];
    value = (value << 8) | b;
    if (b != 0xFF) allOnes = false;
  }
  return _Size(value, width, isUnknown: allOnes);
}
