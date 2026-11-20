# Security policy

AutoFlowMate controls the mouse and keyboard, reads and sends e-mail, and uses passwords stored in
the Windows Credential Manager, so security reports are taken seriously.

## Reporting a problem

Please **do not open a public issue** for a security problem. Report it privately through
GitHub: open the repository's **Security** tab and choose **Report a vulnerability**.

Please include what is affected, how to reproduce it, and what an attacker could do. You will get
a reply within 7 days. Once a fix is released, the report may be published with credit to you,
if you wish.

## Supported versions

Only the latest release receives security fixes.

## Things that are by design

- A flow can do anything the signed-in Windows user can do. Run only flows you trust, just as you
  would run only programs you trust.
- Secrets are stored in the Windows Credential Manager of the current user and are never written
  to flow files or logs.
