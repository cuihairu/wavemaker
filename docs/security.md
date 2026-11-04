---
title: Security & Isolation
---

# Security and Isolation

- IPC between Bridge and Supervisor uses locally permission-protected channels.
- Every Swell uses its own short-lived credentials or random token.
- IPC file permissions allow only the corresponding service account.
- Remote control commands require authentication and authorization.
- Tokens, passwords, or account credentials are never written to logs.
- Backend configuration is separated from game account credentials.
- Plugin/Bridge processes run with least privilege.
- Container resource limits are optional hardening, not the only source of security boundaries.
- Externally exposed Controller APIs must have authentication, authorization, audit, and rate limiting.
- High-risk actions such as remotely cancelling a Swell or cleaning up bots must record the operator and the reason.
