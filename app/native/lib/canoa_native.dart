// The C ABI of crates/canoa-ffi, and nothing above it: one job is one `canoa` command running in this process.
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

/// The ABI this code was written for; a library of another is refused.
const int canoaAbiVersion = 1;

// One table with crates/canoa-ffi/src/exports.rs; app/test/host_library_test.dart compares them.
const int statusOk = 0;
const int statusBadArgument = 1;
const int statusUnknownJob = 2;
const int statusFailed = 3;
const int statusPanic = 4;

/// A call the library refused.
final class CanoaNativeError implements Exception {
  const CanoaNativeError(this.call, this.status);
  final String call;
  final int status;
  @override
  String toString() => 'the host library refused $call: status $status';
}

/// What a job has said since it was last read.
final class JobSaid {
  const JobSaid(this.out, this.err, this.dropped, this.exit);
  final List<String> out, err;

  /// How many lines nobody read in time.
  final int dropped;

  /// Its status, once it has ended and given up everything it held.
  final int? exit;
}

/// The library, loaded.
final class CanoaNative {
  CanoaNative._(DynamicLibrary library, this.path)
    : _abi = library.lookupFunction<Uint32 Function(), int Function()>('canoa_ffi_abi'),
      _start = library.lookupFunction<
        Int32 Function(Pointer<Uint8>, Size, Pointer<Uint64>),
        int Function(Pointer<Uint8>, int, Pointer<Uint64>)
      >('canoa_ffi_job_start'),
      _read = library.lookupFunction<
        Int32 Function(Uint64, Pointer<Pointer<Uint8>>, Pointer<Size>),
        int Function(int, Pointer<Pointer<Uint8>>, Pointer<Size>)
      >('canoa_ffi_job_read'),
      _textFree = library.lookupFunction<Void Function(Pointer<Uint8>, Size), void Function(Pointer<Uint8>, int)>(
        'canoa_ffi_text_free',
      ),
      _stop = library.lookupFunction<Int32 Function(Uint64), int Function(int)>('canoa_ffi_job_stop'),
      _free = library.lookupFunction<Int32 Function(Uint64), int Function(int)>('canoa_ffi_job_free');

  /// Where it was loaded from.
  final String path;
  final int Function() _abi;
  final int Function(Pointer<Uint8>, int, Pointer<Uint64>) _start;
  final int Function(int, Pointer<Pointer<Uint8>>, Pointer<Size>) _read;
  final void Function(Pointer<Uint8>, int) _textFree;
  final int Function(int) _stop;
  final int Function(int) _free;

  /// Where the built application carries it: beside the executable on Windows, in its `lib` directory on Linux, in its
  /// framework on macOS. Named in full, so that no other directory is searched.
  static String beside(String executable) {
    final directory = File(executable).parent.path;
    if (Platform.isWindows) return '$directory\\canoa_ffi.dll';
    if (Platform.isMacOS) return '${File(directory).parent.path}/Frameworks/canoa_native.framework/canoa_native';
    return '$directory/lib/libcanoa_ffi.so';
  }

  /// Loads the library at [path], else the one the running application carries.
  ///
  /// Throws [ArgumentError] when it cannot be loaded, and [StateError] for a library of another ABI.
  factory CanoaNative.open({String? path}) {
    final where = path ?? beside(Platform.resolvedExecutable);
    final native = CanoaNative._(DynamicLibrary.open(where), where);
    final found = native._abi();
    if (found != canoaAbiVersion) {
      throw StateError('the host library at $where has ABI $found, and this app needs $canoaAbiVersion');
    }
    return native;
  }

  /// Starts the command [request] names, and answers its job number.
  int start(Map<String, Object?> request) {
    final bytes = utf8.encode(jsonEncode(request));
    final text = malloc<Uint8>(bytes.isEmpty ? 1 : bytes.length);
    final job = malloc<Uint64>();
    try {
      text.asTypedList(bytes.length).setAll(0, bytes);
      final status = _start(text, bytes.length, job);
      if (status != statusOk) throw CanoaNativeError('start', status);
      return job.value;
    } finally {
      malloc.free(text);
      malloc.free(job);
    }
  }

  /// What job [job] has said since the last read.
  JobSaid read(int job) {
    final text = malloc<Pointer<Uint8>>();
    final length = malloc<Size>();
    try {
      final status = _read(job, text, length);
      if (status != statusOk) throw CanoaNativeError('read', status);
      final Object? value;
      try {
        value = jsonDecode(utf8.decode(text.value.asTypedList(length.value)));
      } finally {
        _textFree(text.value, length.value);
      }
      final said = value! as Map<String, Object?>;
      return JobSaid(
        (said['out']! as List<Object?>).cast<String>(),
        (said['err']! as List<Object?>).cast<String>(),
        said['dropped']! as int,
        said['exit'] as int?,
      );
    } finally {
      malloc.free(text);
      malloc.free(length);
    }
  }

  /// Closes job [job]'s input, and returns at once.
  void stop(int job) {
    final status = _stop(job);
    if (status != statusOk) throw CanoaNativeError('stop', status);
  }

  /// Forgets job [job], closing its input if it has not ended.
  void free(int job) {
    final status = _free(job);
    if (status != statusOk) throw CanoaNativeError('free', status);
  }
}
