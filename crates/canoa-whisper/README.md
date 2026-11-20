# canoa-whisper

Canoa's package for whisper.cpp: the Store program that installs its speech-to-text command line and one model (each checked
against its publisher's SHA-256), the Store type that lists it, a page of its models, and the speech offer a host makes of the
installed copy, so dictation runs on the host's own processor and the audio goes nowhere else.

A build that does not carry it has no whisper.cpp program, models or offer. Not published; `canoa-host` carries it. Every export
is listed in [`AGENTS.md`](AGENTS.md).
