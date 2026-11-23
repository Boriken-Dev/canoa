# canoa-piper

Canoa's package for Piper: the Store program that installs its text-to-speech command line, the Store type that lists it, a page of its voices (an English and
a Spanish one, each installed on its own, each file checked against a SHA-256 Canoa measured), and the
`speech_output` provider that reads text aloud with the installed copy, so replies are read on the host's own computer and the
text goes nowhere else.

A build that does not carry it has no Piper program, voices or provider. Not published; `canoa-host` carries it. Every export is
listed in [`AGENTS.md`](AGENTS.md).
