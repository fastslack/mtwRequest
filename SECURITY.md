# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | Yes                |

## Reporting a Vulnerability

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please report them via email:

**security@mtwrequest.dev**

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

## Response Timeline

| Action                    | Timeframe    |
|---------------------------|--------------|
| Acknowledgment            | 48 hours     |
| Initial assessment        | 5 days       |
| Fix development           | 14 days      |
| Public disclosure          | 30 days      |

## Disclosure Policy

- We will work with you to understand and address the issue
- We will credit you in the security advisory (unless you prefer anonymity)
- We ask that you do not publicly disclose the vulnerability until we have released a fix

## Scope

This policy applies to all code in the `mtw-request` repository, including:

- All Rust crates (`crates/`)
- Language bindings (`bindings/`)
- Frontend SDKs (`packages/`)

## Out of Scope

- Third-party modules from the marketplace (report to module author)
- Issues in dependencies (report upstream, but let us know)
