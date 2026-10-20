# Synthetic TLS test data

These certificates and private keys were generated locally for the isolated
TLS-trust regression test (`tests/tls_trust.rs`). They are not user
credentials and must never be used outside tests:

- `ca.pem` — synthetic test CA that signs `localhost.pem` and
  `wronghost.pem`.
- `untrusted-ca.pem` — unrelated synthetic CA used to prove an untrusted
  chain is rejected.
- `localhost.pem` / `localhost.key` — SAN `DNS:localhost`,
  `IP:127.0.0.1`.
- `wronghost.pem` / `wronghost.key` — trusted chain with SAN
  `DNS:wrong.example.invalid`, used for the hostname-verification case.

The issuing private keys were discarded after signing; no external service,
real provider or user secret is involved.
