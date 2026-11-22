# Security policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private vulnerability reporting instead:
the repository's **Security** tab → **Report a vulnerability**.

Include the version (`termigator --version`), the platform, what an attacker can do and the steps to
reproduce. Remove real host names, user names, keys and passwords from anything you send.

This is a small project maintained in spare time: expect a first reply within about a week. A fix is
released as soon as it is ready, and the advisory is published with it.

## Scope

Of particular interest:

- the password vault (`enc:` values in the configuration) and how secrets reach `ssh`, `sshpass` or the
  askpass helper (they must never appear in argv, logs or error messages);
- command lines built from configuration or imported files (MobaXterm, PuTTY, Remmina, `ssh_config`):
  argument or shell injection;
- file transfers and the Files panel (paths from the remote side);
- the local control socket and the private runtime directory.

Problems in the external tools Termigator starts (OpenSSH, FreeRDP, TigerVNC, …) belong to those projects.
