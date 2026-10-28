# Config

One declarative catalog. Everything else the server needs is either a secret (`05-deployment.md`)
or lives on the client.

## `slot` (owner: `storage`, prefix `slot`)

| field | type | meaning |
|---|---|---|
| max_size_bytes | int64 | refused above this at `PresignUploadRequest` |

Seed, `config/base/storage/slot.json`, namespace `default`:

| name | max_size_bytes |
|---|---|
| full | 26214400 |
| file | 26214400 |

Retention is not config here: it is the bucket's one lifecycle rule, 28 days after upload for
everything in it, a platform setting on the environment's bucket. Thumbnails are not files: they
live inside the object's value, capped by the client at 64 KiB (`06-object-types.md`).

## `storage_quota` (owner: `storage`, prefix `squo`)

| field | type | meaning |
|---|---|---|
| max_bytes_per_168h | int64 | `PresignUploadRequest` refuses when the group's uploads of the last 168 hours plus this one would exceed it |

Seed, `config/base/storage/storage_quota.json`, namespace `default`: one entry, `default`,
`21474836480` (20 GiB).

## Not config

- Object types and their schemas: client-defined; the server stores a number.
- The 672-hour object window, the 1 MiB value cap and the purge worker policies: constants in the
  nanoservice, identical across replicas by construction (Directive invariant 4).
- Push payload shapes: constants in `device`.
