# Security Policy

## Reporting a vulnerability

Corros is a young language — treat it as **not production-hardened** for
untrusted input. Do **not** open a public issue for security problems.

Report vulnerabilities privately to the maintainer:

- **Email**: vishalbabuyt04@gmail.com
- **GitHub**: [cococopi](https://github.com/cococopi)

Please include:

1. The Corros version (`corros -v`) and how it was built
2. A minimal `.cro` program that triggers the issue
3. The expected vs. actual behavior
4. Whether the issue affects the interpreter (`src/`), the self-hosting code
   (`selfhost/`), or the CLI

## What to expect

- **Acknowledgement** within 5 business days.
- **Status updates** at least every 2 weeks until resolution.
- **Coordinated disclosure**: we'll agree on a timeline before public
  disclosure.

## Known areas of caution

- The VM performs no sandboxing: Corros scripts can read files (via the `read`
  builtin), so don't run untrusted scripts with elevated privileges.
- Deeply recursive programs may exhaust the Rust stack (the VM tracks its own
  call depth, but Rust-side recursion in builtins is bounded by the host).
- Numbers are IEEE-754 doubles; expect floating-point behavior.
