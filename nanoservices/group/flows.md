# group flows

## `user_purge` lifecycle

TODO: the state machine of this type — the phases in `status`, what moves
each transition, what the reconcile pass checks level by level, how
teardown confirms absence (the Directive §3 invariant 6).

```mermaid
stateDiagram-v2
    [*] --> pending
    pending --> converged: reconcile pass
    converged --> deleting: mark_deleted
    deleting --> [*]: absence confirmed → Delete
```

## `group_purge` lifecycle

TODO: the state machine of this type — the phases in `status`, what moves
each transition, what the reconcile pass checks level by level, how
teardown confirms absence (the Directive §3 invariant 6).

```mermaid
stateDiagram-v2
    [*] --> pending
    pending --> converged: reconcile pass
    converged --> deleting: mark_deleted
    deleting --> [*]: absence confirmed → Delete
```

## Edge cases

| # | Trigger | Code path | Behaviour |
|---|---|---|---|
| 1 | TODO | | |
