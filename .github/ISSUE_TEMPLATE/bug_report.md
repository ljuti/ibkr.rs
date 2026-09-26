---
name: Bug report
about: Something the client gets wrong — a command failing, a wrong number, an unhelpful error
title: ""
labels: bug
assignees: ""
---

<!--
Please do not paste anything sensitive: no bearer token, no Flex token, no
private key, no statement export, no account number. `ibkr configure --show`
masks the token, but read it over before pasting.
-->

## What happens

<!-- The command, and what came back. Include the whole error, including the
     `caused by:` lines — they carry the real reason. -->

```console
$ ibkr …
```

## What should happen

## Steps to reproduce

1.
2.

## Environment

```console
$ ibkr --version
$ ibkr configure --show -o table      # token is masked
```

- How is the client configured: config file / environment variables / flags?
- Gateway version or commit, if you know it (`GET /health` reports the broker
  server version).
- Was the gateway reachable at the time? `ibkr health` and, if the failure
  smells like the broker rather than the client, `ibkr contracts search AAPL`
  (IBKR error `10159` means the gateway's broker session is degraded — the
  gateway's issue, not this client's).

## Anything else

<!-- If this is about a Flex report or the store, say which report and whether
     `ibkr store status` shows the sections you expect. -->
