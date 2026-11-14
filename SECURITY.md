# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use the repository's
**Security → Report a vulnerability** form (GitHub private vulnerability
reporting) and include:

- affected version and platform;
- a description of the impact;
- reproduction steps or a proof of concept;
- whether credentials, local files or the update channel are involved.

You should receive an acknowledgement within seven days. Please allow time for
a fix before publishing details.

## Scope

Steam Press stores Steam credentials locally (AES-256-GCM, key in
`settings.ini`) and can replace its own GUI executable during an update. The
security review and threat model are in
[docs/security-review.md](docs/security-review.md).

The local vault protects against casual file disclosure, not against an
attacker who can read and write the whole user profile. Automatic updates rely
on the integrity of GitHub releases and SHA-256 checksums; release signing is
tracked separately.

## Supported versions

Security fixes are applied to the latest release and `main`. Older 0.x
versions are not maintained.
