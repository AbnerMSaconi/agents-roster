# Release signing key

Public key of the Agents Roster release signature, in the format read by `ssh-keygen -Y verify`
(`release-signers`).

| | |
|---|---|
| Identity | `release@agents-roster` |
| Namespace | `agents-roster-release` |
| Algorithm | ED25519 |
| Fingerprint | `SHA256:dpdoGJ1Mq+9V9egi99c4bLb8fbgLPRsPX4evWh5Kp0c` |

Verify a release file (needs its `.sig` next to it):

```sh
scripts/verify-release.sh agents-roster-<version>-linux-x86_64.tar.gz
```

The private key is held only by the author and is never stored in CI. Releases published before the
first signed one carry only the SHA-256 file and the Sigstore build provenance attestation.
