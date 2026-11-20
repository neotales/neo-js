---
title: neo-js
description: Use the neojs command-line tool or embed QuickJS in a Rust application.
---

neo-js is a Rust library and command-line host for QuickJS. It runs JavaScript
modules and strips types from TypeScript scripts. Built-in `neo:` modules
provide utilities such as path parsing, plus optional host access for tasks
such as reading files or running child processes.

- [Run a script](/guides/run/) with the command-line tool.
- [Embed the runtime](/runtime/embedding/) in a Rust application.
- [Read the API reference](/api/) for module exports and global signatures.

neo-js is not a sandbox. Review [host permissions](/runtime/permissions/)
before granting a script access to host resources.
