# Conformance vectors

`vectors/*.json` describe what a conforming consumer of the gateway's Flex
report payload must do: decode it, partition its levels of detail, store it, and
answer the same questions from the result. **The file is the specification** —
this repository ships one implementation of it (`tests/conformance.rs`), the
gateway's own tests can run the same cases, and a port in another language must
reproduce them.

Each rule in here was learned against a real statement and cost time to find
(see each case's `why`). They are data rather than prose so they cannot drift
from the code: `cargo test --test conformance` (part of `just ci`) fails the
moment an implementation stops honouring one.

## Format

```jsonc
{
  "broker": "ibkr-flex",
  "cases": [
    {
      "name": "kebab-case-case-name",
      "why": "the rule being pinned, and where it came from",
      "report": "vector",              // report name used for the primary sync
      "raw": { /* the gateway payload, exactly as it arrives */ },
      "expect": {
        "ingest": {                    // SyncStats for the primary sync
          "executions": 2, "lots": 1, "cashTransactions": 1,
          "skipped": 1, "cashDated": 1
        },
        "payload": [                   // field assertions on the decoded payload
          { "path": "trades.1.openClose", "equals": "C;O" }
        ],
        "extraSyncs": [                // synced after the primary one, before queries
          { "report": "narrow-window", "raw": { /* ... */ } }
        ],
        "queries": [                   // store-level assertions: SQL and exact rows
          { "sql": "SELECT COUNT(*) FROM round_trips", "rows": [[2]] }
        ]
      }
    }
  ]
}
```

- Paths in `payload` are dotted; numbers index arrays. `null` means the key is
  absent or null on the wire — IBKR emits empty attributes for fields that do
  not apply, and the gateway maps blank enum values to `null`.
- Numbers compare numerically, so `2` and `2.0` are the same answer.
- Rows compare exactly and in order: use `ORDER BY` when order matters, and
  `WHERE` when a case has several rows that look alike.

## Adding a case

1. Write it against a payload that actually occurred — a bug report's response,
   a change in the gateway, a fresh statement. Synthetic accounts and amounts;
   never a real account number or a real figure.
2. Assert the smallest set of facts that would have caught the original
   mistake. A vector that only restates the code is not worth its maintenance.
3. Run `cargo test --test conformance`.

## Adding a vector file

One file per contract, named after the payload's source (`ibkr-flex.json` for
the gateway's Flex report envelope). A second gateway surface — the OpenAPI
document, a streaming envelope — gets its own file and its own runner entry.
