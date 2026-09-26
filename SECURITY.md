# Security

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository
("Security" → "Report a vulnerability", or the Security tab's advisory form).
That keeps the report private until there is a fix.

If you cannot use it, open an issue that contains **no sensitive detail** and
say you have a private report to share; we will arrange a channel.

Helpful in a report:

- the version (`ibkr --version`) and how it was built;
- the exact command, and the error output including the `caused by:` lines;
- the output of `ibkr configure --show -o table` — the token is masked there,
  but still check before pasting: **never include a bearer token, a private
  key, a Flex token, or a statement export**.

This is a hobby-scale open-source project: there is no SLA, but security reports
are looked at first.

## What this software holds

| Secret | Where it lives | Notes |
|---|---|---|
| Gateway bearer token | config file (`~/.config/ibkr/config.toml`, mode `0600`) or `IBKR_GATEWAY_TOKEN` | Treat it as a password: the gateway authorises whatever the token allows, including order placement. Rotate it on the gateway, then re-run `ibkr configure`. |
| Client private key (mTLS) | wherever you generated it, referenced by path | The client only reads it. Never commit it: `.gitignore` covers `*-key.pem`, `*.key`, `.certs/` and `tmp/`. |
| Trade data | the store (`ibkr.db` by default) | Account numbers, positions, realized P&L, cash movements. Treat the file as sensitive; it is gitignored, and `store query` opens it read-only. |

## What is protected, and what is not

- **No telemetry, no phone-home.** The client talks only to the gateway URL you
  configure.
- **The token is never printed.** `configure --show` masks it, and errors never
  carry it.
- **The config file is written `0600`** and `configure --show` warns when it is
  readable by others.
- **It is plaintext on disk.** There is no OS keyring integration; a process
  that can read your files can read the token.
- **A token passed as `--token` is visible in `ps`** to other users on the same
  machine. Prefer the config file or the environment variable.
- **`--tls-skip-verify` disables certificate verification.** Development only:
  it makes the connection interceptable.
- **Read-only mode refuses order mutations locally** (`--read-only`,
  `IBKR_READ_ONLY`, or the config file's `read-only`). It is a guard rail, not a
  boundary: a client can be rebuilt without it, so the gateway's own
  `GATEWAY_READ_ONLY` remains the authority.
- **Mutations are not retried.** Order placement and cancellation are sent
  exactly once, whatever the status, and prompt for confirmation unless `--yes`
  is given.

## Out of scope

- The IBKR gateway itself is a separate repository — report its issues there,
  including anything about how it terminates TLS or enforces read-only mode.
- Interactive Brokers' own services, and the TWS / IB Gateway applications.
- Anything that requires an attacker to already hold your token or your private
  key, unless the client leaks it in a way this document says it does not.
