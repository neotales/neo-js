---
title: "Scheduler configuration"
description: "Generate systemd and Windows Task Scheduler configuration text."
---

Use `neo:os/schedulers/systemd` to generate systemd units and
`neo:os/schedulers/windows` to generate Windows Task Scheduler XML. Both are
synchronous JavaScript text generators and can run on any host supported
by neo-js. The optional `neojs-schedulers` crate provides both canonical
names and the deprecated combined alias `neo:scheduler`. The CLI registers
the crate. Core alone provides none of the three imports.

No scheduler specifier needs a permission flag, grants host authority, or
installs a global. The generators do not read or write files, create
directories, change permissions, run a shell or service manager, or
install, register, start, or enable jobs. Writing and installing the
output requires separate authority.

## Generate systemd units

```js
import { systemdUnits } from "neo:os/schedulers/systemd";

const { service, timer } = systemdUnits({
	name: "backup",
	description: "Nightly backup",
	program: "/usr/local/bin/backup",
	args: ["--full", "My Data"],
	workingDirectory: "/var/lib/backup",
	user: "backup",
	schedule: { kind: "daily", at: "02:30" },
	persistent: true,
});

// Writing these strings is a separate decision:
// import { writeText } from "neo:fs"; // requires --allow-fs
// writeText("/etc/systemd/system/backup.service", service);
// writeText("/etc/systemd/system/backup.timer", timer);
```

The service is a `Type=oneshot` unit. The timer activates `<name>.service` and
is wanted by `timers.target`; the generated service has no `[Install]` section
because the timer, not the service, is the unit to enable. A system unit
normally belongs in `/etc/systemd/system` and needs administrator authority; a
user unit normally belongs in `~/.config/systemd/user`. Installing or
reloading either is outside this module.

## Generate Task Scheduler XML

```js
import { windowsTask } from "neo:os/schedulers/windows";

const xml = windowsTask({
	name: "Backup",
	description: "Nightly backup",
	startDate: "2026-11-18",
	program: "C:\\Program Files\\Neo\\backup.exe",
	args: ["--full", "C:\\Data Folder"],
	workingDirectory: "C:\\Program Files\\Neo",
	schedule: { kind: "daily", at: "02:30" },
	principal: {
		userId: "CONTOSO\\svc-backup",
		logonType: "Password",
		runLevel: "LeastPrivilege",
	},
});
```

`windowsTask` returns XML with the declaration
`<?xml version="1.0" encoding="UTF-16"?>`. Write the string as UTF-16LE with a
byte-order mark, the usual Task Scheduler export encoding, or replace the
declaration with one that matches the bytes you actually write.
`WINDOWS_XML_ENCODING` is `"UTF-16LE"`. The module returns text, not bytes, and
performs no encoding. It does not register the task; registration and the
`Password` credential are the caller's responsibility, and this module never
accepts or stores a password.

## Choose a schedule

The supported schedule is deliberately small:

| Schedule                                         | systemd                             | Task Scheduler                          |
| ------------------------------------------------ | ----------------------------------- | --------------------------------------- |
| `{kind: "daily", at: "HH:MM"}`                   | `OnCalendar=*-*-* HH:MM:00`         | `ScheduleByDay` with `DaysInterval=1`   |
| `{kind: "weekly", at: "HH:MM", weekdays: [...]}` | `OnCalendar=Mon,Wed *-*-* HH:MM:00` | `ScheduleByWeek` with `WeeksInterval=1` |

`HH` is 00 through 23 and `MM` is 00 through 59; seconds, one-shot schedules,
monthly schedules, and interval schedules are not generated. Weekdays are the
lowercase three-letter names `mon` through `sun`, must be distinct, and are
emitted in Monday-through-Sunday order. Unknown kinds, a daily schedule that
also carries `weekdays`, and a weekly schedule without `weekdays` are rejected
rather than approximated.

Time zones are not converted. An `OnCalendar` expression without a time-zone
suffix uses the local time of the machine running the timer. A Task Scheduler
`StartBoundary` is emitted without a time-zone suffix and Task Scheduler
applies its own local-time interpretation. The two targets can therefore mean
different instants around a daylight-saving transition, and neither target's
missed-run or wall-clock behaviour is modelled. `persistent: true` emits
systemd's `Persistent=true`, which catches up a missed OnCalendar run when the
timer is next activated. The Windows XML deliberately omits `<Settings>`, so
Task Scheduler's registration defaults apply; the module does not claim that
the two catch-up policies are equivalent.

`startDate` is required for `windowsTask` and is a Gregorian `YYYY-MM-DD` date
from 1601 through 9999. It is written verbatim as the trigger's
`StartBoundary` date and is not computed or adjusted by this module. The
systemd side has no corresponding date.

## Arguments and escaping

For systemd, `ExecStart=` is generated as one double-quoted item per argument,
and systemd's own command-line parser, not a shell, turns the line back into
an argv. Whitespace, quotes, backslashes, semicolons, percent signs, and
dollar signs inside an argument do not split it or run a command. `%` is
doubled because systemd expands specifiers; `$` is doubled because ExecStart
performs environment-variable substitution; `\` and `"` are backslash-escaped.
A literal argument consisting of one semicolon, a program containing `$`, and
control characters are rejected because their ExecStart interpretation is
ambiguous. No shell is used and no shell quoting is attempted. A caller that
wants a shell must name it explicitly as the program, and then the arguments
belong to that shell's language, which this module does not escape.

`Description=`, `User=`, and `WorkingDirectory=` are emitted without
surrounding quotes and accept a smaller character set than systemd itself:
`%`, backslashes, quotation marks, control characters, and leading or trailing
whitespace are rejected. This avoids specifier expansion, escape, and
line-continuation ambiguity. `WorkingDirectory` must be an absolute POSIX path.
`User` must be a simple name or a decimal numeric ID; it is not looked up, and
the module does not set `Group=`, supplementary groups, capabilities, or
`DynamicUser=`. With no `user`, the unit uses the service manager's default
(root for a system unit, the invoking user for a user unit).

For Windows, `<Command>` holds the program path by itself and the module does
not wrap it in quotation marks. `<Arguments>` holds the remaining arguments
joined by one space, with each argument quoted according to the Microsoft C/C++
runtime `CommandLineToArgvW` convention. That convention is used by many
programs but not by every program; in particular it is not `cmd.exe` or
PowerShell quoting. A program that parses its command line differently needs
its own argument representation, which this module does not invent.

Windows paths must be absolute drive paths (`C:\...`) or UNC paths
(`\\server\share\...`) with backslash separators. Relative paths, forward
slashes, `.` and `..` components, empty components, invalid Windows filename
characters, percent signs, and device paths such as `\\?\...` and `\\.\...`
are rejected. The Task Scheduler schema's 260-character `pathType` limit is
enforced for the program and working directory. No path is checked for
existence, and other Windows path-length and permissions policy is not
enforced.

`principal` is optional. When present, it requires `userId` and one of the
Task Scheduler logon types `InteractiveToken`, `Password`, `S4U`, or
`InteractiveTokenOrPassword`, and optionally `LeastPrivilege` or
`HighestAvailable` as `runLevel`. Those four names are the complete
`LogonType` enumeration in the authoritative XSD; `Group` and
`ServiceAccount` are rejected rather than emitted. `InteractiveToken` runs
only for an already logged-on user, `Password` needs credentials supplied
separately at registration, and `S4U` stores no password and has no network
access; those are Task Scheduler's semantics, not this module's.
`HighestAvailable` requests the highest privileges available to the account.
Omitting `principal` omits `<Principals>` so registration applies its own
default; that default is not asserted here. A systemd `User=` only changes the
service process identity; it does not grant or remove the authority of the
administrator who installs the unit.

## Errors and verification

Every rejected value throws `SchedulerError` with a stable `code` and the
`field` that failed. Messages never include the rejected value, so a secret in
an argument is not copied into an error. The codes are `SCHEDULER_TYPE` (wrong
JavaScript type), `SCHEDULER_VALUE` (invalid value), `SCHEDULER_SCHEDULE`
(invalid schedule), `SCHEDULER_UNSUPPORTED` (unsupported or not safely
representable), and `SCHEDULER_LIMIT` (a documented size bound).

Each specifier has its own `SchedulerError` constructor, including the legacy
specifier. Use `instanceof` with the constructor imported from the same
specifier as the function that threw the error. The `code` and `field` have
the same meanings across the three modules.

Every options object must be a plain object (prototype `Object.prototype`) or
a null-prototype record whose own properties are enumerable data properties.
Accessors, own symbol properties, non-enumerable own properties, unknown keys,
arrays, class instances, and objects with another prototype are rejected.
Validation reads property descriptors rather than `object[key]`, so an
accessor is rejected without being invoked. Only own values are used: an
inherited enumerable property is rejected, an inherited value never satisfies
a known option, and accepted values are copied onto a null-prototype record
before use. A transparent Proxy over a conforming record is
indistinguishable and may be accepted; a Proxy can run its traps during
inspection. `args` and `weekdays` are read by own index; an accessor or
non-enumerable index is rejected. Other own properties on those arrays are
ignored and cannot alter the generated schedule or argv. A misspelled or
unsupported key such as `interval` or `groupId` is never silently ignored.
Shape violations report `SCHEDULER_TYPE` for a non-object, array, or non-plain
prototype, and `SCHEDULER_UNSUPPORTED` for accessors, own symbols,
non-enumerable own properties, inherited enumerable properties, or unknown
keys. The exported bounds are
`MAX_SYSTEMD_LINE_BYTES` and `MAX_WINDOWS_COMMAND_LINE`; there is also a
per-argument bound and an internal argument-count bound. A Proxy trap can
still throw its own error.

The generated Task Scheduler XML declares schema version 1.3, the fixed value
in the authoritative XSD. `CalendarTrigger` follows the XSD sequence
(`Enabled` before `StartBoundary`). Weekly output emits `DaysOfWeek` before
`WeeksInterval`; the XSD's `weeklyScheduleType` is `xs:all`, so either order is
schema-valid and this is a documented convention, not a validity requirement.
`Actions/@Context` is emitted only when a principal with `id="Author"` is
present, because the schema's `ContextKeyRef` requires the reference to
resolve.

The repository tests run generated units through `systemd-analyze verify` and
validate generated UTF-16LE XML against the pinned Task Scheduler 1.3 XSD with
`xmllint --schema` on temporary files, when those tools are installed. The
schema fixture is reproduced from Microsoft Learn with provenance and license
in `crates/neojs-schedulers/tests/fixtures/` (`task-scheduler-1.3.xsd` and
`NOTICE.md`). These checks validate the generated files.
They do not test installation or verify that Windows registers or runs a task.

## Migrate from `neo:scheduler`

`neo:scheduler` is deprecated. No release is currently scheduled to remove it.
It still exports both sets of generators and constants under that name, with
the same output and error contract:

```js
import { systemdUnits, windowsTask } from "neo:scheduler";
```

Use the canonical names in new code. Move systemd imports to
[`neo:os/schedulers/systemd`](/api/modules/os/schedulers/systemd/) and Windows
imports to [`neo:os/schedulers/windows`](/api/modules/os/schedulers/windows/).
If a script uses both, split the import and match errors to the corresponding
module. See the [legacy declarations](/api/modules/scheduler/) for the
compatibility exports.

## Embed scheduler generators

For a checkout next to your crate, add both dependencies:

```toml
[dependencies]
neojs-core = { path = "../neo-js/crates/neo-js" }
neojs-schedulers = { path = "../neo-js/crates/neojs-schedulers" }
```

```rust
use neojs_core::{NeoJs, Permissions, RuntimeOptions};
use neojs_schedulers::{MODULE_NAMES, with_schedulers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let options = with_schedulers(RuntimeOptions::new())?.module_names(MODULE_NAMES)?;
	let runtime = NeoJs::new_with_options(Permissions::default(), options)?;
	runtime.run_module(
		"main.js",
		r#"import {systemdUnits} from "neo:os/schedulers/systemd";
		   import {windowsTask} from "neo:os/schedulers/windows";
		   import {systemdUnits as legacyUnits} from "neo:scheduler";
		   const schedule = {kind: "daily", at: "09:30"};
		   const unit = {name: "backup", program: "/bin/true", schedule};
		   if (systemdUnits(unit).service !== legacyUnits(unit).service)
		     throw Error("legacy output differs");
		   if (!windowsTask({name: "Backup", startDate: "2026-11-18",
		                     program: "C:\\backup.exe", schedule}).includes("<Task"))
		     throw Error("expected task XML");"#,
	)?;
	Ok(())
}
```

`with_schedulers` registers all three names with `ExtensionPermission::None`.
Use `neojs_schedulers::schedulers_extension()` if you assemble the registry
yourself. Registering the extension does not change an explicit allowlist.
If you need only one name, pass only its exact constant to `module_names`:

| Removed `RuntimeModule` variant   | Replacement constant                    |
| --------------------------------- | --------------------------------------- |
| `RuntimeModule::SchedulerSystemd` | `neojs_schedulers::SYSTEMD_MODULE_NAME` |
| `RuntimeModule::SchedulerWindows` | `neojs_schedulers::WINDOWS_MODULE_NAME` |
| `RuntimeModule::Scheduler`        | `neojs_schedulers::LEGACY_MODULE_NAME`  |

Admitting either canonical name admits only that target, not the other
target or the legacy alias. Admitting the legacy name grants access to both
generators through `neo:scheduler` only. Registering or admitting `neo:os`
has no effect on the schedulers. Static and dynamic imports of a single
specifier share a module instance. Different specifiers do not share an
instance or a `SchedulerError` constructor.

`ModuleAllowlist::all()` contains built-ins only. Register the extension
and add exact names, or use
`ModuleAllowlist::all_with_extensions(options.extensions())` to admit all
registered extensions. Unrestricted `RuntimeOptions::default()` admits
registered names but does not register them itself.

For a host that registers only `neojs-schedulers`, compose declarations with
`--with schedulers`. This selects all three declarations. Any explicit
runtime allowlist still controls each import independently. See
[Select declarations](/runtime/typescript/#select-declarations).
