---
title: "Network downloads"
description: "Synchronous HTTP downloads, publication, progress, and errors."
---

`neo:fetch` provides one synchronous function, `download(url, path, options?)`.
It requires `--allow-net`; without that permission the module is absent and an
import fails. Every download also requires `--allow-fs`, because it writes the
destination. JavaScript argument validation, including any option getters or
proxy traps, can run or throw before the native filesystem permission check.
Once validation passes, missing `--allow-fs` produces `FETCH_PERMISSION`;
the download itself does not parse the URL, invoke progress, or perform file
or network I/O before this check. A relative destination path is resolved
against the runtime's current working directory, not the host process
directory. The native callback is private to the module; there is no
`__neoFetch*` global.

```js
import { download } from "neo:fetch";

const result = download("https://example.test/release.tar.gz", "release.tar.gz", {
	maxBytes: 32 * 1024 * 1024,
	timeoutMs: 60_000,
	overwrite: true,
	progress(received, total) {
		// total is a number when Content-Length is known, otherwise null.
	},
});
console.log(`saved ${result.bytes} bytes with status ${result.status}`);
```

`--allow-net` is broad outbound network authority under the current user's
network access, not a sandbox or an allowlist. Loopback, link-local, private,
and cloud-metadata addresses are reachable. A redirect may change the host,
port, or scheme, including an HTTPS-to-HTTP downgrade. There is no IP
filtering, DNS pinning, redirect-host restriction, or SSRF protection. If
untrusted input chooses the URL, enforce an allowlist or isolate the process at
the OS layer; do not rely on this module to do it.

`download` blocks the JavaScript thread until the body is received and
published. There is no asynchronous form and no streaming API. Transport and
file work run on a joined worker thread, so a host may call it from a Tokio
runtime; progress callbacks still run synchronously on the calling thread.
At most 32 download workers are active process-wide. Further calls fail with
`FETCH_NETWORK` rather than waiting, including recursively nested downloads;
the limit avoids deadlock and unbounded worker creation.

## URL, transport, and status

Only absolute `http` and `https` URLs with a host and no userinfo are accepted;
other schemes and any userinfo are rejected with `FETCH_INVALID_URL` before a
connection. URLs are checked again on every redirect, including empty `@` and
empty-password forms, before the next request. This is deliberate: a client could
otherwise turn URL userinfo into an `Authorization: Basic` header. The request
is a `GET`; besides transport defaults (`Host`, `User-Agent`, and `Accept`), no
headers are sent. There is no `Authorization` header, cookie jar, proxy, or
automatic decompression; environment proxy discovery is explicitly disabled.
The URL is never logged or included in an error message. neo-js follows at
most five redirects; a chain that reaches the limit reports `FETCH_REDIRECT`.
A final status outside 2xx reports `FETCH_STATUS` with the numeric status in
`error.status`; the response body is not consumed or saved. The result's
`status` is the final status.

HTTPS uses rustls with the bundled Mozilla webpki roots and certificate-chain
and hostname verification enabled; TLS 1.2 and TLS 1.3 are supported. There is
no switch to disable verification, so a TLS-intercepting proxy whose
certificate does not chain to a bundled root causes the request to fail; the
host's own trust store is not consulted. DNS resolution and connection time are
bounded by the same overall timeout on a best-effort basis; an OS resolver can
still block.

## Options and bounds

| option      | default | accepted range            |
| ----------- | ------- | ------------------------- |
| `maxBytes`  | 64 MiB  | integer 0 through 512 MiB |
| `timeoutMs` | 30,000  | integer 1 through 600,000 |
| `overwrite` | `false` | boolean                   |
| `progress`  | none    | function                  |

Omitting `options` uses the defaults. For an explicit options object, only
own properties count; inherited or `undefined` options use their defaults.
Own getters can run during validation, as can traps on a proxy options object.

`timeoutMs` covers the network operation, including connection, redirects,
header reads, body reads, and time spent inside the progress callback; it does
not cover the final local sync or publication. `maxBytes` applies both to a
declared `Content-Length` (checked before the body is read)
and to the bytes actually streamed. A body that exceeds the bound reports
`FETCH_LIMIT`; an over-limit body is not written past the bound and temporary
cleanup is attempted. Because automatic decompression is disabled, the
limit and the saved bytes describe the response body as received; a server
that applies `Content-Encoding` without being asked yields the encoded bytes.
A response that ends before its declared `Content-Length` reports
`FETCH_TRUNCATED`. Non-integer, out-of-range, or wrongly typed options throw
`TypeError`.

## Destination and atomic publication

`path` must name a file with an existing parent directory; relative paths use
the runtime working directory, not the host process directory, and report
`FETCH_CWD` when that directory is unavailable. Missing parents are not created
and report `FETCH_PATH`. The body is streamed to a newly created private temp directory
(`.neo-download-*.tmp`, mode `0700` on Unix) in the destination's parent
directory, into a file opened with mode `0600` (both modes are subject to the
process umask). neo-js keeps the open file handle and checks that the pathname
still names that file before publication and cleanup. The temp directory
is also identity-checked before removal (device/inode on Unix, a same-file
handle on other platforms; Windows is unverified), and removal is
non-recursive. The file is synchronized with `sync_all` and then published. The destination is never
written directly, so a status error, size limit, truncated body, callback
failure, or local write error leaves an existing destination untouched on
normal error paths, assuming trusted paths and callbacks as described below.
Temporary cleanup is best-effort: if removal fails, a temporary directory
and possibly its file can remain; the original download error is preserved.
The destination directory
itself is not synchronized, so durability of the directory entry across a
power loss is filesystem-dependent.

With `overwrite: false`, an existing destination is rejected before the
network request and publication uses a no-clobber hard link. That is atomic on
Unix and is the intended Windows behavior (unverified); it also stays on the
same filesystem because the temporary directory is in the destination
directory. A filesystem without hard-link support reports `FETCH_IO` and
leaves the destination untouched. With `overwrite: true`, publication is a
rename that replaces the destination atomically on Unix; Windows behavior is
unverified. On Unix the destination receives the temporary file's mode, so a
new download is private (`0600` before umask) and replacing an existing
destination does not preserve its old permissions. Windows ACL inheritance is
unverified. The preflight existence check is advisory; the no-clobber publish
is authoritative, so a destination that appears during the download is not
replaced when `overwrite` is false. When `overwrite` is true, another writer
can be replaced.

The parent directory and its path must be trusted against modifications by
other users and same-user processes, including progress callbacks. A user
able to modify the parent can rename or replace even a `0700` directory.
Identity checks may detect substitution, but on all platforms they are
separate from hard-link, rename and removal operations. Substitution can race
with publication, changing the destination, or with cleanup, deleting a
replacement. Portable standard-library APIs do not exclude these races.
An abrupt process termination can leave a `.neo-download-*.tmp` directory; it
is never the destination and never replaces one.

## Progress

`progress(received, total)` is called once with `received` `0` before the body
is read and again after each chunk with the cumulative bytes received. `total`
is the declared `Content-Length` or `null` when it is unknown. The callback is
synchronous and runs on the JavaScript thread. If it throws, the download
aborts with `FETCH_CALLBACK` instead of propagating the original exception.
With permissions granted, a direct `async` function throws `FETCH_CALLBACK_ASYNC`
before invocation or I/O. Bound or wrapped callbacks can run before their
return value is checked. Returning a Promise or thenable also throws
`FETCH_CALLBACK_ASYNC` synchronously. A returned Promise is marked handled
without awaiting or cancelling its work. For other objects and functions, a
callable data property `then` (own or inherited) is rejected. A `then` accessor
or a proxy encountered during this lookup is also rejected without running a
getter or trap. Other return values are ignored.

On callback failure, `download` does not publish the destination. It attempts
temporary cleanup and joins the worker before throwing the original download
error, even if cleanup fails. This does not roll back side effects of the
callback itself.
A callback may start another `download`; each call has independent temporary
state. Callback time counts against `timeoutMs`; the deadline is checked after
every callback and before publication. Callbacks must be trusted not to modify
the destination, parent path, or temporary entries.

The TypeScript declaration rejects inferred Promise and thenable returns,
but TypeScript allows a value-returning function to be assigned to `() => void`.
For example, annotating options as plain `DownloadOptions` can hide a Promise
return from the checker. The runtime check still applies.

## Errors

Wrong JavaScript argument types throw `TypeError`. Other failures throw an
`Error` with a stable `code`:

| code                   | meaning                                                                                                                        |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `FETCH_PERMISSION`     | `--allow-fs` was not granted                                                                                                   |
| `FETCH_INVALID_URL`    | not an accepted `http`/`https` URL with a host and no userinfo                                                                 |
| `FETCH_OPTION`         | an option value failed native validation                                                                                       |
| `FETCH_STATUS`         | the final status was outside 2xx; `error.status` is set                                                                        |
| `FETCH_REDIRECT`       | the redirect limit was reached                                                                                                 |
| `FETCH_TIMEOUT`        | the overall timeout expired                                                                                                    |
| `FETCH_NETWORK`        | DNS, connection, TLS, protocol, or body-read failure                                                                           |
| `FETCH_IO`             | local temporary-file, sync, link, or rename failure                                                                            |
| `FETCH_LIMIT`          | the body exceeded `maxBytes`                                                                                                   |
| `FETCH_TRUNCATED`      | the body ended before its declared length                                                                                      |
| `FETCH_EXISTS`         | the destination exists and `overwrite` is false                                                                                |
| `FETCH_PATH`           | the destination path or parent is invalid                                                                                      |
| `FETCH_CWD`            | the runtime working directory is unavailable for a relative destination                                                        |
| `FETCH_TEMP`           | the temporary pathname no longer names the open file                                                                           |
| `FETCH_CALLBACK`       | the progress callback threw                                                                                                    |
| `FETCH_CALLBACK_ASYNC` | the progress callback is async, returned a Promise or thenable, or returned a value whose `then` could not be safely inspected |

Messages are fixed and never include the URL, a credential, a path, or
server-provided text. The module adds no credentials and starts no shell.

Declarations: [`neo:fetch`](/api/modules/fetch/).
