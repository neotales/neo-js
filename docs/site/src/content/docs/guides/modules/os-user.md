---
title: Account and process identity
description: Read bounded account and process identity with an explicit opt-in.
---

The CLI registers `neo:os/user` and requires `--allow-user` to import it.
The module is independent of `neo:os` and adds no globals. Grant access
only to scripts you trust with account names, profile paths, and user and
group IDs. Avoid logging that information or using it to authorize operations.

| Function                | What it reports                                                                                                                                                                    |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `currentUserSync()`     | The account record for the caller's **real UID**: `uid`, `primaryGid`, `username`, and `homeDir`. This is not the effective user or interactive login account.                     |
| `homeDirSync()`         | The account's profile directory, without consulting `HOME` or checking that the directory exists.                                                                                  |
| `processIdsSync()`      | The caller's real and effective UIDs and GIDs, sampled separately without an account lookup.                                                                                       |
| `accountGroupIdsSync()` | The real user's OS-reported **account memberships**, including the account's primary GID, sorted numerically and deduplicated. These are not the live process's group credentials. |

Linux and macOS expose unsigned 32-bit IDs. On Windows and other platforms
the module can be imported, but its functions throw `USER_UNSUPPORTED`.
Lookups are synchronous, uncached, and non-atomic. The account service (NSS
on Linux) can consult remote sources and block without a deadline. neo-js
bounds only its own buffers and retries: at most seven account attempts
and 256 KiB of account buffer, and one group query with room for 4,096 IDs.
No credentials are changed. A temporary-directory API is pending because
platform temp selection can reveal environment values that `--allow-user`
does not grant.

Unlike Go's `os.UserHomeDir()`, `homeDirSync()` returns the account profile
path, not an environment value. The account's primary GID may differ from
either current process GID.

Save as `check-profile.js` and run `cargo run -p neojs -- --allow-user check-profile.js`.
It reports the outcome without printing identity data or the caught error:

```js
import { homeDirSync } from "neo:os/user";

try {
	homeDirSync();
	console.log("Account profile is available.");
} catch (error) {
	if (error instanceof Error && error.code === "USER_NOT_FOUND") {
		console.log("No account record is available.");
	} else if (error instanceof Error && error.code === "USER_UNSUPPORTED") {
		console.log("Account lookup is unsupported on this platform.");
	} else {
		console.error("Account profile lookup failed.");
	}
}
```

Lookup errors have fixed messages without account data, paths, OS error
strings, or a stack. See the [API reference](/api/modules/os/user/) for the
codes each function can throw.

| Code               | Meaning                                                      |
| ------------------ | ------------------------------------------------------------ |
| `USER_NOT_FOUND`   | The OS reported no account for the caller's real UID.        |
| `USER_ENCODING`    | A returned account name or profile path was not valid UTF-8. |
| `USER_LIMIT`       | A neo-js buffer, retry, or allocation limit was reached.     |
| `USER_LOOKUP`      | The OS lookup failed or returned malformed data.             |
| `USER_UNSUPPORTED` | There is no backend for the platform.                        |

## Embed account lookups

Link `neojs-os-user`, register it, grant `Permissions::user`, and include
its exact module name in any explicit allowlist. Registration grants no
permission and does not register `neojs-os`.

```rust
use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_os_user::{MODULE_NAME, with_os_user};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = with_os_user(RuntimeOptions::new())?.module_names([MODULE_NAME])?;
    let runtime = NeoJs::new_with_options(
        Permissions { user: true, ..Permissions::default() },
        options,
    )?;
    runtime.run_module("main.js", "import 'neo:os/user';")?;
    Ok(())
}
```

Use `--with os-user` when [composing TypeScript declarations](/runtime/typescript/).
