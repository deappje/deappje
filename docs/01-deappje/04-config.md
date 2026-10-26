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
| thumb | 204800 |
| file | 26214400 |

Retention is not config here: it is the bucket's lifecycle rules per prefix (`full` and `file`
two days; `thumb` none), a platform setting on the environment's bucket.

## `storage_quota` (owner: `storage`, prefix `squo`)

| field | type | meaning |
|---|---|---|
| max_bytes_per_48h | int64 | `PresignUploadRequest` refuses when the group's uploads of the last 48 hours plus this one would exceed it |

Seed, `config/base/storage/storage_quota.json`, namespace `default`: one entry, `default`,
`21474836480` (20 GiB).

## Not config

- Object types and their schemas: client-defined; the server stores a number.
- The 48-hour object window, the 1 MiB value cap and the purge worker policies: constants in the
  nanoservice, identical across replicas by construction (Directive invariant 4).
- Push payload shapes: constants in `device`.
