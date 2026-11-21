// Unit tests for AgentVendor, StoreExtension, ProgramStore, ModelStore, speech classes/helpers, and forms.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { BadValue, Unsupported } from 'guatiao';
import { library, s } from 'guatiao/provide';

import {
  AccountSpeech,
  AgentVendor,
  FORM_KEY,
  ModelStore,
  ProgramStore,
  SpeechOutput,
  SpeechProvider,
  StoreExtension,
  formMeta,
  section,
  ui,
  when,
} from '../src/index.js';
import { CommandSpeech, PCM_S16LE, fill, languageCode, wavHeader } from '../src/speech.js';

function fakeSpawner(destroyed) {
  return {
    destroy() {
      destroyed.push(true);
    },
  };
}

test('AgentVendor answers defaults, overrides, and dynamic info()', () => {
  class Quill extends AgentVendor {
    static agent = 'quill';
    static title = 'Quill';
    static vendor = 'quill-labs';
    static standards = [{ id: 'quill-key', title: 'Quill key' }];
    static modes = [{ id: 'code', title: 'Code' }];
    static mark = '<svg/>';
  }
  const info = new Quill().describe();
  assert.equal(info.agent, 'quill');
  assert.equal(info.title, 'Quill');
  assert.equal(info.vendor, 'quill-labs');
  assert.deepEqual(info.standards, [{ id: 'quill-key', title: 'Quill key' }]);
  assert.deepEqual(info.modes, [{ id: 'code', title: 'Code' }]);
  assert.equal(info.mark, '<svg/>');
  assert.equal(info.look, undefined);

  class ComputedVendor extends AgentVendor {
    static agent = 'computed';
    static title = 'Unused';
    info() {
      return { agent: 'computed', title: 'Computed' };
    }
  }
  assert.deepEqual(new ComputedVendor().describe(), { agent: 'computed', title: 'Computed' });
  assert.deepEqual(library('vendor_test', '1', { providers: [Quill] }).kinds, ['agent_vendor']);
});

test('StoreExtension, ProgramStore, and ModelStore describe cleanly and refuse unimplemented slots', () => {
  class Notes extends StoreExtension {
    static title = 'Notes';
    static publisher = 'Example';
    static types = [{ id: 'notes', title: 'Notes' }];
  }
  assert.deepEqual(new Notes().describe(), {
    title: 'Notes',
    publisher: 'Example',
    types: [{ id: 'notes', title: 'Notes', stores: [] }],
  });

  class BaseProgramStore extends ProgramStore {}
  assert.throws(() => new BaseProgramStore().info(), Unsupported);
  assert.throws(() => new BaseProgramStore().program({}, {}, {}), Unsupported);
  assert.throws(() => new BaseProgramStore().latest({}, {}), Unsupported);
  assert.throws(() => new BaseProgramStore().plan({}, {}), Unsupported);
  assert.throws(() => new BaseProgramStore().apply({}, {}, {}, {}), Unsupported);

  class BaseModelStore extends ModelStore {}
  assert.throws(() => new BaseModelStore().info(), Unsupported);
  assert.throws(() => new BaseModelStore().list({}, {}), Unsupported);
  assert.throws(() => new BaseModelStore().act({}, {}), Unsupported);
  assert.throws(() => new BaseModelStore().keep({}, {}), Unsupported);
  assert.throws(() => new BaseModelStore().service({}, {}), Unsupported);
  assert.deepEqual(library('store_test', '1', { providers: [Notes] }).kinds, ['store_extension']);
});

test('AccountSpeech, SpeechProvider, SpeechOutput, and CommandSpeech helpers behave as specified', () => {
  class CloudSpeech extends AccountSpeech {
    static services = [
      {
        id: 'cloud',
        title: 'Cloud',
        standard: 'openai-api',
        address: 'https://api.example/v1',
        url: 'https://api.example/v1/audio/transcriptions',
        credential: 'api_key',
      },
    ];
  }
  assert.deepEqual(new CloudSpeech().describe(), {
    services: [
      {
        id: 'cloud',
        title: 'Cloud',
        standard: 'openai-api',
        address: 'https://api.example/v1',
        url: 'https://api.example/v1/audio/transcriptions',
        credential: 'api_key',
        summary: '',
        location: '',
        models: [],
      },
    ],
  });

  class Mic extends SpeechProvider {
    static title = 'Mic';
    static encodings = [PCM_S16LE];
  }
  assert.deepEqual(new Mic().describe(), {
    title: 'Mic',
    encodings: ['pcm_s16le'],
    partials: false,
    hints: [],
  });
  const gone = [];
  assert.throws(() => new Mic().open({}, fakeSpawner(gone)), Unsupported);

  class Speaker extends SpeechOutput {
    info() {
      return { id: 'speaker', title: 'Speaker' };
    }
  }
  assert.deepEqual(new Speaker().describe(), { id: 'speaker', title: 'Speaker' });
  assert.throws(() => new Speaker().voices({}, {}), Unsupported);
  assert.throws(() => new Speaker().say({}, {}), Unsupported);

  const header = wavHeader({ rate: 22050, channels: 2 });
  assert.equal(header.length, 44);
  assert.equal(String.fromCharCode(...header.slice(0, 4)), 'RIFF');
  assert.equal(fill('--lang={language|auto} --other={missing}', { language: '' }), '--lang=auto --other={missing}');
  assert.equal(languageCode('es-PR'), 'es');
  assert.equal(languageCode('en_US'), 'en');
  assert.equal(CommandSpeech.id, 'speech_command');
});

test('forms helpers place sections, UI hints, visibility rules, and guatiao schema keywords', () => {
  const schema = s.object({
    command: s.array(s.string(), ui('run')),
    beam: s.integer({ default: 5, ...ui('run') }),
  });
  const meta = formMeta({
    sections: [section('run', 'Run', 'Command settings')],
    fields: { beam: { unit: 'beams', visibleWhen: when('translate', true) } },
  });
  const FormSpeech = class extends SpeechProvider {
    static title = 'Form Speech';
    static encodings = [PCM_S16LE];
    static config = schema;
    static meta = meta;
  };
  assert.equal(FORM_KEY, 'form');
  assert.deepEqual(ui('run'), { 'x-section': 'run' });
  assert.equal(/** @type {any} */ (schema.json).properties.beam['x-section'], 'run');
  assert.deepEqual(FormSpeech.meta.form, {
    sections: [{ id: 'run', title: 'Run', description: 'Command settings' }],
    fields: { beam: { unit: 'beams', visibleWhen: { field: 'translate', equals: true } } },
  });
  assert.equal(FormSpeech.config, schema);
  assert.ok(new BadValue('check') instanceof Error);
});
