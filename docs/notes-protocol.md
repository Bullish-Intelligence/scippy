# Shared note facts

Run `scippy notes`. Send one JSON object per line on standard input.
The worker replies with one JSON object per line on standard output.
It reads no vault files and runs no Markdown scanner.

Each request has `format_version: 1`, `command`, and an optional `id`.
Each reply echoes `id` and has `format_version: 1` and `ok`.
A successful reply stores its result in `value`.
A refusal stores `error.kind`, `error.message`, and the unchanged `generation`.
Malformed messages do not end the worker. End input to stop it.

Commands:

- `reset`: supply `base_generation`, `profile`, `sources`, `attachments`, and optional `policy`.
- `update`: supply `base_generation`, the same `profile`, `changes`, and optional replacement `attachments`.
- `snapshot`: return all current records.
- `ping`: return the current generation, including zero before reset.

Reset and update return `generation`, `changed` (records by source path), and `removed` (source paths).
The initial reset expects generation zero. Each successful mutation adds one.
A reset validates its replacement before it discards the current index.
An update validates the entire batch before it changes any lookup bucket.

The profile has `format_version: 1`, `extractor_version: 1`, and `pack_digests`.
The digest map must contain `markdown`, `markdown_inline`, and `yaml`.
Each digest has the form `sha256:` followed by 64 lowercase hexadecimal digits.
A profile or policy change requires a reset.

A source has `path`, `sha256`, `text`, `note`, and optional `status` (default `indexed`).
The hash binds the exact UTF-8 source bytes. Paths are project-relative paths.
The `Note` structure is the serialized `scippy_loci::Note` type.
Spans are half-open UTF-8 byte ranges in the original source.
Decoded YAML values can use the complete scalar span as source support.
Consumers must not treat these spans as replacement instructions.
The trusted producer owns syntax, marker ownership, and extraction semantics.
Scippy validates source binding, lengths, paths, limits, and span boundaries.

Use status `unavailable` or `skipped` with null `text` and `note` for unreadable or refused sources.
These sources retain their names. References to them report `skipped-target` and have no anchor proof.
A change has `path`, `base_sha256` (null for creation), and `source` (null for deletion).
Use one batch for a rename's deletion and creation.

The worker limits one note to two mebibytes and the source text set to 256 mebibytes.
It accepts at most 50,000 source paths, 100,000 attachment paths, and 256 mebibytes per message.
Ordinary updates trace name lookups, including misses, and invalidate affected records.
Attachment and availability changes conservatively rebuild the lookup maps.
The caller owns persistence, process recovery, source discovery, and publication transactions.
Replay committed facts after a worker failure. Publish the returned delta atomically with its source facts.

The full index and metalayer commands keep their existing scanner and output contracts.
