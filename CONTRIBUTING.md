# Contributing to mtwRequest

First off, thank you for considering contributing to mtwRequest! Every contribution matters, whether it's a bug report, feature suggestion, documentation improvement, or code change.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How Can I Contribute?](#how-can-i-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Features](#suggesting-features)
  - [Your First Code Contribution](#your-first-code-contribution)
  - [Pull Requests](#pull-requests)
- [Development Setup](#development-setup)
- [Building the Project](#building-the-project)
- [Running Tests](#running-tests)
- [Code Style](#code-style)
- [Commit Messages](#commit-messages)
- [Module Development](#module-development)
- [Where to Ask Questions](#where-to-ask-questions)
- [Recognition](#recognition)

## Code of Conduct

This project and everyone participating in it is governed by the [mtwRequest Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior as described in the Code of Conduct.

## How Can I Contribute?

### Reporting Bugs

Before creating a bug report, please check existing [issues](https://github.com/fastslack/mtwRequest/issues) to avoid duplicates. When you create a bug report, use the [bug report template](https://github.com/fastslack/mtwRequest/issues/new?template=bug_report.yml) and include as many details as possible.

**Good bug reports include:**
- A clear and descriptive title
- Steps to reproduce the issue
- Expected vs. actual behavior
- Your environment (OS, Rust version, mtwRequest version)
- Relevant log output or screenshots

### Suggesting Features

Feature requests are welcome! Use the [feature request template](https://github.com/fastslack/mtwRequest/issues/new?template=feature_request.yml) and describe:

- The problem you're trying to solve
- Your proposed solution
- Alternatives you've considered
- Whether you'd be willing to implement it

### Your First Code Contribution

Not sure where to start? Look for issues labeled:

- **`good first issue`** -- straightforward tasks for newcomers
- **`help wanted`** -- issues where we'd appreciate community help
- **`docs`** -- documentation improvements

### Pull Requests

1. Fork the repository and create your branch from `main`
2. Follow the [development setup](#development-setup) instructions
3. Make your changes, following our [code style](#code-style)
4. Add or update tests as needed
5. Ensure all tests pass
6. Update documentation if you changed APIs
7. Submit your pull request using the [PR template](.github/PULL_REQUEST_TEMPLATE.md)

## Development Setup

### Prerequisites

| Tool | Version | Required |
|------|---------|----------|
| Rust | 1.75+ | Yes |
| Node.js | 18+ | For frontend SDKs |
| Python | 3.10+ | For Python bindings |
| PHP | 8.1+ | For PHP bindings |

### Getting Started

```bash
# Clone the repository
git clone https://github.com/fastslack/mtwRequest.git
cd mtw-request

# Verify Rust toolchain
rustup show

# Install required components
rustup component add rustfmt clippy
```

## Building the Project

### Rust core (all crates)

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Build a specific crate
cargo build -p mtw-core
```

### Frontend SDKs

```bash
cd packages/client
npm install
npm run build

# Or for React SDK
cd packages/react
npm install
npm run build
```

## Running Tests

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p mtw-core

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name
```

## Code Style

We use standard Rust formatting and linting tools. All code must pass these checks before merging:

```bash
# Format code
cargo fmt

# Check formatting (CI uses this)
cargo fmt --check

# Run linter
cargo clippy -- -D warnings

# Run linter on all targets
cargo clippy --all-targets --all-features -- -D warnings
```

**General guidelines:**
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Write doc comments for all public items
- Keep functions focused and small
- Prefer returning `Result` over panicking
- Use `thiserror` for error types in library crates

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | A new feature |
| `fix` | A bug fix |
| `docs` | Documentation changes |
| `style` | Formatting, missing semicolons, etc. (no code change) |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `perf` | Performance improvement |
| `test` | Adding or updating tests |
| `build` | Build system or external dependency changes |
| `ci` | CI configuration changes |
| `chore` | Other changes that don't modify src or test files |

### Scopes

Use the crate name as the scope when applicable:

```
feat(mtw-ai): add streaming support for Ollama provider
fix(mtw-transport): handle WebSocket reconnection on network change
docs(mtw-sdk): add module development guide
```

## Module Development

If you're building a module for the mtwRequest ecosystem, check out:

- [Module Development Guide](docs/creating-modules.md)
- [AI Agent Guide](docs/ai-agents.md)
- [SDK Documentation](crates/mtw-sdk/)

Modules use the `MtwModule` trait system. The `mtw-sdk` crate provides proc macros (`#[mtw_module]`, `#[mtw_handler]`, etc.) to simplify development.

```bash
# Scaffold a new module
mtw generate module my-module

# Scaffold an AI agent
mtw generate agent my-agent
```

## Where to Ask Questions

- **GitHub Discussions** -- for questions, ideas, and general conversation: [Discussions](https://github.com/fastslack/mtwRequest/discussions)
- **Issues** -- for bugs and feature requests only

Please don't use issues for support questions. Use Discussions instead.

## Recognition

All contributors are recognized in our releases. Significant contributions are highlighted in the [CHANGELOG](CHANGELOG.md).

Thank you for helping make mtwRequest better!
