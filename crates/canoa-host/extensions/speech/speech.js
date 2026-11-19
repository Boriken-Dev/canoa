// The speech provider that ships with the host, written with the Canoa SDK: `speech_command` runs a program the person named, audio on its
// standard input and text on its standard output. The module is the library: its id and version are its package's, its providers the
// classes it exports.
import { CommandSpeech } from './sdk/speech.js';

export class Command extends CommandSpeech {}
