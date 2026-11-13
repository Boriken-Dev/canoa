import 'dart:js_interop';

@JS('localStorage.getItem')
external JSString? _get(JSString key);

@JS('localStorage.setItem')
external void _set(JSString key, JSString value);

@JS('localStorage.removeItem')
external void _remove(JSString key);

const _key = 'canoa-site-documents';

String? load() {
  try {
    return _get(_key.toJS)?.toDart;
  } on Object {
    return null;
  }
}

void save(String? folder) {
  try {
    folder == null ? _remove(_key.toJS) : _set(_key.toJS, folder.toJS);
  } on Object {
    // Without storage the folder is asked again next time.
  }
}
