# Security Policy

lunaRun downloads and runs server software on developers' machines, so we take security seriously.

## Supported versions

Until 1.0, only the latest released version receives security fixes.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through GitHub's **"Report a vulnerability"** button (Security tab → Advisories), or
email **hello@digitallybird.com** with "lunaRun security" in the subject.

Please include steps to reproduce, affected version and OS, and the impact you see.
We aim to acknowledge reports within 72 hours.

## Scope highlights

Especially interesting: download/checksum/registry verification bypasses, archive path traversal,
file deletion outside the lunaRun data directory, privilege escalation through the elevation helper,
and exposure of services beyond `127.0.0.1`.
