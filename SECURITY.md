# Security

Please report security problems privately through GitHub: the repository's **Security** tab,
**Report a vulnerability**. If that button is missing, open an issue asking for a private
contact, without details. Do not put the problem itself in a public issue.

Useful to know when assessing a report:

- Marionette runs game scripts and jobs from the project folder. Treat a game project from an
  untrusted source like any other code you would run.
- Provider API keys are read from environment variables or a keys file outside every project,
  are never printed, and a keys file inside a project is refused.
- Paid provider calls need a spend cap that only a person can set (zero by default), and the
  engine's MCP server cannot raise it.
- Every commit is scanned for secrets in CI (`.gitleaks.toml`).
