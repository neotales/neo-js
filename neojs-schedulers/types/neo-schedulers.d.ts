// Optional scheduler generators for neojs-schedulers. Referenced by the
// compose-types selection; core declarations deliberately omit these modules.

/**
 * Pure, synchronous generation of systemd unit text. Requires no permission
 * flag and grants no host capability.
 *
 * No function reads or writes a file or directory, changes permissions,
 * creates a process, runs a shell, calls a service manager, registers a task,
 * or enables anything. The returned strings are inert until a caller with
 * separate authority writes and installs them. A caller that uses `neo:fs` to
 * write them needs `--allow-fs`; a caller that uses a deployment tool needs
 * whatever authority that tool requires. This module never installs. This is
 * not isolation from other code in the same realm.
 *
 * A schedule is `{kind: "daily", at: "HH:MM"}` or
 * `{kind: "weekly", at: "HH:MM", weekdays: [...]}`. `HH` is 00 through 23 and
 * `MM` is 00 through 59; seconds are not accepted. Weekdays are the lowercase
 * three-letter names `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, and `sun`.
 * They are emitted in Monday-through-Sunday order. Daily becomes
 * `OnCalendar=*-*-* HH:MM:00`; weekly becomes
 * `OnCalendar=Mon,Wed *-*-* HH:MM:00`. Other kinds and contradictory fields
 * are rejected.
 *
 * Time zones are not converted. An OnCalendar expression without a time-zone
 * suffix uses the timer's local time. Daylight-saving transitions and
 * missed runs are not modelled here.
 *
 * systemd arguments are emitted as one quoted `ExecStart` item per argument,
 * and systemd's own parser, not a shell, turns the line back into an argv.
 * `%` is doubled for specifier expansion, `$` is doubled for environment
 * substitution, and `\` and `"` are backslash-escaped. A lone `;` argument, a
 * program containing `$`, control characters, and unpaired surrogates are
 * rejected. `Description`, `User`, and `WorkingDirectory` accept a smaller
 * character set than systemd itself: `%`, backslashes, quotation marks,
 * control characters, and leading or trailing whitespace are rejected.
 * `WorkingDirectory` must be an absolute POSIX path. `User` must be a simple
 * name or a decimal numeric ID; it is not looked up.
 *
 * Every options object must be a plain object (prototype `Object.prototype`)
 * or a null-prototype record whose own properties are enumerable data
 * properties. Accessors, own symbol properties, non-enumerable own properties,
 * unknown keys, arrays, class instances, and objects with another prototype
 * are rejected. Validation reads property descriptors rather than
 * `object[key]`, so an accessor is rejected without being invoked. Only own
 * values are used: an inherited enumerable property is rejected, an inherited
 * value never satisfies a known option, and accepted values are copied onto a
 * null-prototype record before use. A transparent Proxy over a conforming
 * record is indistinguishable and may be accepted; a Proxy can run its traps
 * during inspection. `args` and `weekdays` are read by own index; an accessor
 * or non-enumerable index is rejected. Other own properties on those arrays
 * are ignored and cannot alter the generated schedule or argv. A misspelled
 * or unsupported key such as `interval` or `groupId` is not silently ignored. Shape violations use `SCHEDULER_TYPE` for a
 * non-object, array, or non-plain prototype, and `SCHEDULER_UNSUPPORTED`
 * for accessors, own symbols, non-enumerable own properties, inherited
 * enumerable properties, or unknown keys.
 *
 * Every string option rejects control characters, unpaired UTF-16 surrogates,
 * and U+FFFE or U+FFFF. Every rejected input throws `SchedulerError` with a
 * stable `code` and the option `field` that failed. Messages never include the
 * rejected value. A getter or Proxy trap on an options object can still throw
 * its own error.
 */
declare module "neo:os/schedulers/systemd" {
  /** Stable failure codes; match on these, never on the message. */
  export type SchedulerErrorCode =
    | "SCHEDULER_TYPE"
    | "SCHEDULER_VALUE"
    | "SCHEDULER_SCHEDULE"
    | "SCHEDULER_UNSUPPORTED"
    | "SCHEDULER_LIMIT";

  /**
   * Thrown for invalid options or unsupported values. Match against the
   * `SchedulerError` exported by the same module as the generator.
   */
  export class SchedulerError extends Error {
    name: "SchedulerError";
    /** Stable code; match on this, never on the message. */
    code: SchedulerErrorCode;
    /** The option path that failed, such as `program` or `args[0]`. */
    field: string;
  }

  /** A lowercase three-letter weekday name. */
  export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

  /** Every day at a local wall-clock time. */
  export interface DailySchedule {
    kind: "daily";
    /** Exactly `HH:MM`, 00:00 through 23:59. */
    at: string;
  }

  /** Selected weekdays at a local wall-clock time. */
  export interface WeeklySchedule {
    kind: "weekly";
    /** Exactly `HH:MM`, 00:00 through 23:59. */
    at: string;
    /** One or more distinct weekdays; emitted in Monday-through-Sunday order. */
    weekdays: readonly Weekday[];
  }

  /** The supported recurring schedules. No one-shot or interval form exists. */
  export type Schedule = DailySchedule | WeeklySchedule;

  /** Options for a systemd service unit. */
  export interface SystemdServiceOptions {
    /**
     * Base unit name without `.service` or `.timer`. ASCII letters, digits,
     * `_`, `.`, and `-` only; must start with a letter or digit; at most 200
     * characters; must not end with a dot or a unit suffix.
     */
    name: string;
    /**
     * Optional `Description=`. Must not contain `%`, `\`, `"`, `'`, control
     * characters, or leading or trailing whitespace.
     */
    description?: string;
    /** Absolute POSIX path to the executable. Spaces are quoted safely. */
    program: string;
    /** Arguments passed directly as separate argv items, not through a shell. */
    args?: readonly string[];
    /** Optional absolute POSIX `WorkingDirectory=`. */
    workingDirectory?: string;
    /** Optional `User=` as a simple name or decimal numeric ID. */
    user?: string;
  }

  /** Options for a systemd timer unit. */
  export interface SystemdTimerOptions {
    /** Same base name as the service unit. */
    name: string;
    /** The recurring calendar schedule. */
    schedule: Schedule;
    /** Emit `Persistent=true`; defaults to `false` and only affects OnCalendar. */
    persistent?: boolean;
  }

  /** Options that produce a matched service and timer pair. */
  export interface SystemdUnitsOptions
    extends SystemdServiceOptions,
      SystemdTimerOptions {}

  /**
   * Generate a `Type=oneshot` systemd service unit. The result ends with one
   * newline and has no `[Install]` section: the timer is the enabled unit.
   *
   * @throws SchedulerError for an invalid or unsupported value.
   */
  export function systemdService(options: SystemdServiceOptions): string;

  /**
   * Generate a systemd timer unit that activates `<name>.service` and is
   * wanted by `timers.target`. The result ends with one newline.
   *
   * @throws SchedulerError for an invalid or unsupported value.
   */
  export function systemdTimer(options: SystemdTimerOptions): string;

  /**
   * Generate a matched service and timer. The caller must install them as
   * `<name>.service` and `<name>.timer`; this function writes nothing.
   *
   * @throws SchedulerError for an invalid or unsupported value.
   */
  export function systemdUnits(options: SystemdUnitsOptions): {
    service: string;
    timer: string;
  };

  /** Maximum UTF-8 byte length of a generated systemd `ExecStart=` line. */
  export const MAX_SYSTEMD_LINE_BYTES: 1048576;
}

/**
 * Pure, synchronous generation of Windows Task Scheduler XML. Requires no
 * permission flag and grants no host capability. It returns text only; it
 * never writes, installs, registers, or runs a task.
 *
 * Uses the same local wall-clock schedules and options record validation as
 * the systemd module. Daily schedules use `ScheduleByDay`/`DaysInterval=1`;
 * weekly schedules use `ScheduleByWeek`/`WeeksInterval=1`. Task Scheduler
 * interprets `StartBoundary` in local time; no time-zone conversion is done.
 *
 * `<Command>` is not quoted. `<Arguments>` uses the Microsoft C/C++ runtime
 * `CommandLineToArgvW` convention, not `cmd.exe` or PowerShell quoting.
 * Write the returned XML as UTF-16LE with a byte-order mark, or replace its
 * `encoding="UTF-16"` declaration to match the bytes you write.
 *
 * Paths must be absolute drive or UNC paths with backslash separators.
 * Relative paths, forward slashes, `.` and `..` components, empty
 * components, invalid filename characters, percent signs, and device paths
 * are rejected. The Task Scheduler XSD's 260-character path limit applies.
 * No path is checked for existence.
 *
 * The generated task declares schema version 1.3. `CalendarTrigger` emits
 * `Enabled` before `StartBoundary`, weekly output emits `DaysOfWeek` before
 * `WeeksInterval`, and `Actions/@Context` is emitted only with a principal.
 */
declare module "neo:os/schedulers/windows" {
  import type { Schedule, SchedulerErrorCode } from "neo:os/schedulers/systemd";
  export type {
    DailySchedule,
    Schedule,
    SchedulerErrorCode,
    Weekday,
    WeeklySchedule,
  } from "neo:os/schedulers/systemd";

  /**
   * Thrown for invalid options or unsupported values. Match against the
   * `SchedulerError` exported by the same module as the generator.
   */
  export class SchedulerError extends Error {
    name: "SchedulerError";
    /** Stable code; match on this, never on the message. */
    code: SchedulerErrorCode;
    /** The option path that failed, such as `program` or `args[0]`. */
    field: string;
  }

  /**
   * Task Scheduler logon type accepted by the authoritative XSD. `Password`
   * sets only the XML value; this module never accepts, handles, or stores a
   * password. `Group` and `ServiceAccount` are not in the XSD enumeration
   * and are rejected.
   */
  export type WindowsLogonType =
    | "InteractiveToken"
    | "Password"
    | "S4U"
    | "InteractiveTokenOrPassword";

  /** Task Scheduler run level. */
  export type WindowsRunLevel = "LeastPrivilege" | "HighestAvailable";

  /** Optional Task Scheduler principal. Omitting it omits `<Principals>`. */
  export interface WindowsPrincipal {
    /** Account name or SID. Not looked up. */
    userId: string;
    logonType: WindowsLogonType;
    runLevel?: WindowsRunLevel;
  }

  /** Options for a Windows Task Scheduler task. */
  export interface WindowsTaskOptions {
    /**
     * Task name, emitted as `\<name>` in `<URI>`. At most 255 characters; no
     * backslash or slash; not `.` or `..`; no leading or trailing whitespace.
     */
    name: string;
    /** Optional `Description`. XML-escaped. */
    description?: string;
    /**
     * First eligible local date as `YYYY-MM-DD`. A Gregorian date from 1601
     * through 9999. Combined with the schedule time and `:00` seconds.
     */
    startDate: string;
    /** Absolute Windows drive or UNC path to the executable. */
    program: string;
    /** Arguments quoted with the C/C++ `CommandLineToArgvW` convention. */
    args?: readonly string[];
    /** Optional absolute Windows `WorkingDirectory`. */
    workingDirectory?: string;
    /** The recurring calendar schedule. */
    schedule: Schedule;
    /** Optional principal; omitting it lets registration apply its defaults. */
    principal?: WindowsPrincipal;
  }

  /**
   * Generate Task Scheduler 2.0 XML. The result ends with one newline and
   * declares `encoding="UTF-16"`; see the module notes for writing it.
   *
   * @throws SchedulerError for an invalid or unsupported value.
   */
  export function windowsTask(options: WindowsTaskOptions): string;

  /** Byte encoding to use when writing `windowsTask` output: UTF-16LE with a BOM. */
  export const WINDOWS_XML_ENCODING: "UTF-16LE";

  /** Maximum Windows command-line length, including the terminating NUL. */
  export const MAX_WINDOWS_COMMAND_LINE: 32767;
}

/**
 * @deprecated Import from `neo:os/schedulers/systemd` or
 * `neo:os/schedulers/windows` instead. Allowlisting this legacy name grants
 * both generators through this name only. All three specifiers are checked
 * separately. Catch `SchedulerError` from the same module as the generator.
 */
declare module "neo:scheduler" {
  import type { SchedulerErrorCode } from "neo:os/schedulers/systemd";

  /** Thrown by the legacy module for invalid options or unsupported values. */
  export class SchedulerError extends Error {
    name: "SchedulerError";
    /** Stable code; match on this, never on the message. */
    code: SchedulerErrorCode;
    /** The option path that failed, such as `program` or `args[0]`. */
    field: string;
  }

  export {
    MAX_SYSTEMD_LINE_BYTES,
    systemdService,
    systemdTimer,
    systemdUnits,
  } from "neo:os/schedulers/systemd";
  export type {
    DailySchedule,
    Schedule,
    SchedulerErrorCode,
    SystemdServiceOptions,
    SystemdTimerOptions,
    SystemdUnitsOptions,
    Weekday,
    WeeklySchedule,
  } from "neo:os/schedulers/systemd";
  export {
    MAX_WINDOWS_COMMAND_LINE,
    WINDOWS_XML_ENCODING,
    windowsTask,
  } from "neo:os/schedulers/windows";
  export type {
    WindowsLogonType,
    WindowsPrincipal,
    WindowsRunLevel,
    WindowsTaskOptions,
  } from "neo:os/schedulers/windows";
}
