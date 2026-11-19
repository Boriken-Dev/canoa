// The catalog sources that ship with the host, written with the Canoa SDK: `command` runs a program the person named and takes what it
// prints, and `read` names a place the Store type reads itself (its own reader fetches a git source or an address; a file on this
// computer is read here). The module is the library: its id and version are its package's, its providers the classes it exports.
import { CommandCatalogSource, ReadCatalogSource } from './sdk/source.js';

export class Command extends CommandCatalogSource {}

export class Read extends ReadCatalogSource {}
