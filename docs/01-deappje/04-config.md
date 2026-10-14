# Config

One declarative catalog. Everything else the server needs is either a secret (`05-deployment.md`)
or lives on the client.

## `blob_kind` (owner: `storage`, prefix `bkind`)

| field | type | meaning |
|---|---|---|
| retention_days | int32 | how long a blob stays in the bucket after upload or last access |
| max_size_bytes | int64 | refused above this at `CreateUploadRequest` |

Seed, `config/base/storage/blob_kind.json`, namespace `default`:

| name | retention_days | max_size_bytes |
|---|---|---|
| full | 30 | 26214400 |
| thumbnail | 3650 | 204800 |
| attachment | 365 | 26214400 |

Read by `storage`'s `CreateUploadRequest` handler (to copy `retention_days` into the blob's spec
and to enforce the cap). Written only by a human editing the seed; the app loads it at boot in one
transaction through `basable-config`. Changing a kind's retention affects blobs created after the
change; existing blobs keep the value in their spec.

## Not config

- Object types and their schemas: client-defined; the server stores a number.
- Tombstone retention (90 days) and worker policy: constants in the nanoservice, identical across
  replicas by construction (Directive invariant 4).
- Push payload shapes: constants in `device`.
