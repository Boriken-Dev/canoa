// A Store extension written in JavaScript: it adds one type, Notes, to the Store. A type's `stores` name the `program_store` providers the
// same library offers; this one has none, so the type lists nothing to install and shows what an extension is made of.
import { StoreExtension } from '../../../../bindings/js/canoa-sdk/src/store.js';

export class Notes extends StoreExtension {
  static title = 'Notes';
  static publisher = 'Example publisher';
  static types = [{ id: 'notes', title: 'Notes', icon: 'extension' }];
}
