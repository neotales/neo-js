/**
 * Pure, synchronous generation of scheduler configuration text.
 *
 * `systemdService`, `systemdTimer`, and `systemdUnits` return systemd unit
 * text. `windowsTask` returns Windows Task Scheduler XML text. No function
 * reads or writes a file, creates a directory, changes permissions, starts a
 * process, runs a shell, calls a service manager, registers a task, or
 * enables anything. The returned strings have no effect until a caller with
 * separate authority writes and installs them.
 *
 * The module needs no permission flag and grants no host capability. It does
 * not install anything on the global object. This is not isolation from other
 * code in the same realm, and it cannot stop a caller from writing or
 * installing the returned text.
 *
 * Scheduled triggers are deliberately narrow. A schedule is either
 * `{kind: "daily", at: "HH:MM"}` or
 * `{kind: "weekly", at: "HH:MM", weekdays: [...]}`. `HH` is 00 through 23 and
 * `MM` is 00 through 59; seconds are not accepted. Weekdays are the lowercase
 * three-letter names `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, and `sun`.
 * They are sorted into Monday-through-Sunday order in the output. A daily
 * schedule becomes `*-*-* HH:MM:00` in systemd and `ScheduleByDay` with a
 * one-day interval in Task Scheduler. A weekly schedule becomes
 * `Mon,Wed *-*-* HH:MM:00` in systemd and `ScheduleByWeek` with a one-week
 * interval in Task Scheduler. Other kinds, and unknown fields that would
 * change a supported kind, are rejected rather than approximated.
 *
 * Time zones are not converted. systemd OnCalendar expressions without a
 * time-zone suffix use the local time of the machine running the timer. A
 * Windows `StartBoundary` is emitted without a time-zone suffix and Task
 * Scheduler applies its own local-time interpretation. The two targets can
 * therefore mean different instants during a daylight-saving transition, and
 * neither target's missed-run or wall-clock behaviour is modelled here.
 *
 * systemd arguments: `ExecStart=` is generated as one quoted item per
 * program argument, so systemd's own command-line parser, not a shell, turns
 * the line back into an argv. Whitespace, quotes, backslashes, semicolons,
 * percent signs, and dollar signs inside an argument do not split it or run a
 * command. `%` is doubled because systemd expands specifiers; `$` is doubled
 * because ExecStart performs environment-variable substitution; `\` and `"`
 * are escaped with a backslash. A literal argument consisting of one
 * semicolon, a program containing `$`, and control characters are rejected
 * because their ExecStart interpretation is ambiguous. No shell is used and
 * no shell quoting is attempted.
 *
 * systemd unit values: `Description=`, `User=`, and `WorkingDirectory=` are
 * emitted without surrounding quotes. Their accepted character sets are
 * intentionally smaller than systemd's: control characters, `%`, backslashes,
 * and quotation marks are rejected, and leading or trailing whitespace is
 * rejected. This avoids specifier expansion, escape, and line-continuation
 * ambiguity. `WorkingDirectory` must be absolute. `User` must be a simple name
 * or a decimal numeric ID; its existence is not checked.
 *
 * Windows arguments: `<Command>` holds the program path by itself. The module
 * does not wrap it in quotation marks. `<Arguments>` holds the remaining
 * arguments joined by one space, with each argument quoted according to the
 * Microsoft C/C++ runtime `CommandLineToArgvW` convention. That convention is
 * used by many programs but not by every program; in particular it is not
 * `cmd.exe` or PowerShell quoting. A program that parses its command line
 * differently needs its own argument representation, which this module does
 * not invent. `windowsTask` writes the XML declaration `encoding="UTF-16"`;
 * a caller must write the string as UTF-16LE with a byte-order mark (the
 * usual Task Scheduler export encoding), or replace the declaration with one
 * that matches the bytes it writes. The module returns text, not bytes, and
 * performs no encoding.
 *
 * Windows paths must be absolute drive paths (`C:\...`) or UNC paths
 * (`\\server\share\...`) with backslash separators. Relative paths, forward
 * slashes, `.` and `..` components, empty components, invalid Windows
 * filename characters, and device paths such as `\\?\...` and `\\.\...` are
 * rejected. The Task Scheduler schema's 260-character `pathType` limit is
 * enforced for the program and working directory. No path is checked for
 * existence, and other Windows path-length and permissions policy is not
 * enforced.
 *
 * The generated task declares schema version 1.3. The authoritative XSD
 * fixes `Task/@version` to 1.3, so the older 1.2 value fails genuine schema
 * validation. `CalendarTrigger` follows the XSD sequence (`Enabled` before
 * `StartBoundary`), weekly output emits `DaysOfWeek` before `WeeksInterval`
 * (the XSD type is `xs:all`, so this is a documented convention rather than a
 * validity requirement), and `Actions/@Context` is emitted only when a
 * principal with `id="Author"` is present because the schema's
 * `ContextKeyRef` requires the reference to resolve.
 *
 * A principal is optional. When present, `LogonType` must be one of
 * `InteractiveToken`, `Password`, `S4U`, or `InteractiveTokenOrPassword`,
 * matching the XSD enumeration. `Group` and `ServiceAccount` are not in that
 * enumeration and are rejected rather than emitted.
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
 * or unsupported key such as `interval` or `groupId` is not silently ignored.
 *
 * Every string option rejects control characters, unpaired UTF-16 surrogates,
 * and U+FFFE or U+FFFF; XML output cannot represent the last two. Every
 * rejected input throws `SchedulerError` with a stable `code` and the option
 * `field` that failed. Messages never include the rejected value, so a secret
 * in an argument is not copied into an error. A getter or Proxy trap on an
 * options object can still throw its own error. Limits are documented by
 * `MAX_SYSTEMD_LINE_BYTES` and `MAX_WINDOWS_COMMAND_LINE`.
 */

const WINDOWS_XML_ENCODING = 'UTF-16LE';
const MAX_SYSTEMD_LINE_BYTES = 1048576;
const MAX_WINDOWS_COMMAND_LINE = 32767;
const MAX_UNIT_NAME_LENGTH = 200;
const MAX_TASK_NAME_LENGTH = 255;
const MAX_DESCRIPTION_LENGTH = 32767;
const MAX_USER_ID_LENGTH = 1024;
const MAX_ARGUMENT_LENGTH = 131072;
const MAX_ARGUMENTS = 65536;
const MAX_POSIX_PATH_LENGTH = 32767;
// Task Scheduler's pathType is maxLength 260, used by Command and
// WorkingDirectory. A longer value would be schema-invalid even though the
// Windows command line itself could hold more.
const MAX_WINDOWS_PATH_LENGTH = 260;

const isArray = Array.isArray;
const isInteger = Number.isInteger;
const getPrototypeOf = Object.getPrototypeOf;
const getOwnPropertyNames = Object.getOwnPropertyNames;
const getOwnPropertySymbols = Object.getOwnPropertySymbols;
const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
const create = Object.create;
const objectPrototype = Object.prototype;
const hasOwn = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
const trim = Function.prototype.call.bind(String.prototype.trim);

const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const SYSTEMD_WEEKDAY = {
  mon: 'Mon',
  tue: 'Tue',
  wed: 'Wed',
  thu: 'Thu',
  fri: 'Fri',
  sat: 'Sat',
  sun: 'Sun',
};
const WINDOWS_WEEKDAY = {
  mon: 'Monday',
  tue: 'Tuesday',
  wed: 'Wednesday',
  thu: 'Thursday',
  fri: 'Friday',
  sat: 'Saturday',
  sun: 'Sunday',
};

// These are the only values the authoritative Task Scheduler 1.3 XSD accepts
// for LogonType. Group and ServiceAccount are not in that enumeration.
const LOGON_TYPES = {
  InteractiveToken: true,
  Password: true,
  S4U: true,
  InteractiveTokenOrPassword: true,
};
const RUN_LEVELS = { LeastPrivilege: true, HighestAvailable: true };

const SYSTEMD_SERVICE_KEYS = [
  'name',
  'description',
  'program',
  'args',
  'workingDirectory',
  'user',
];
const SYSTEMD_TIMER_KEYS = ['name', 'schedule', 'persistent'];
const SYSTEMD_UNITS_KEYS = [
  'name',
  'description',
  'program',
  'args',
  'workingDirectory',
  'user',
  'schedule',
  'persistent',
];
const WINDOWS_TASK_KEYS = [
  'name',
  'description',
  'startDate',
  'program',
  'args',
  'workingDirectory',
  'schedule',
  'principal',
];
const SCHEDULE_KEYS = ['kind', 'at', 'weekdays'];
const PRINCIPAL_KEYS = ['userId', 'logonType', 'runLevel'];

/** Thrown for every rejected option value. */
class SchedulerError extends Error {
  constructor(code, field, message) {
    super(message);
    this.name = 'SchedulerError';
    this.code = code;
    this.field = field;
  }
}

function fail(code, field, message) {
  throw new SchedulerError(code, field, message);
}

function isAllowed(allowed, key) {
  for (let index = 0; index < allowed.length; index += 1) {
    if (allowed[index] === key) {
      return true;
    }
  }
  return false;
}

function recordField(field) {
  return field === '' ? 'options' : field;
}

function optionField(field, key) {
  return field === '' ? key : field + '.' + key;
}

// Accept only plain or null-prototype records whose own string properties are
// enumerable data properties. Reading descriptors instead of `object[key]`
// means an accessor is rejected without being invoked. Copying the data values
// into a null-prototype record guarantees that every later read sees an own
// value and cannot fall through to Object.prototype.
function validateRecord(value, field, allowed) {
  if (typeof value !== 'object' || value === null || isArray(value)) {
    fail(
      'SCHEDULER_TYPE',
      recordField(field),
      'must be a plain object or a null-prototype record',
    );
  }
  const prototype = getPrototypeOf(value);
  if (prototype !== objectPrototype && prototype !== null) {
    fail(
      'SCHEDULER_TYPE',
      recordField(field),
      'must be a plain object or a null-prototype record',
    );
  }
  for (const key in value) {
    if (!hasOwn(value, key)) {
      fail(
        'SCHEDULER_UNSUPPORTED',
        optionField(field, key),
        'inherited properties are not allowed',
      );
    }
  }
  if (getOwnPropertySymbols(value).length !== 0) {
    fail('SCHEDULER_UNSUPPORTED', recordField(field), 'must not have own symbol properties');
  }
  const names = getOwnPropertyNames(value);
  const record = create(null);
  for (let index = 0; index < names.length; index += 1) {
    const key = names[index];
    const path = optionField(field, key);
    const descriptor = getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined) {
      // A Proxy can report a key and then omit its descriptor.
      fail('SCHEDULER_TYPE', recordField(field), 'changed while being inspected');
    }
    if (hasOwn(descriptor, 'get') || hasOwn(descriptor, 'set')) {
      fail('SCHEDULER_UNSUPPORTED', path, 'must be an enumerable data property, not an accessor');
    }
    if (!descriptor.enumerable) {
      fail('SCHEDULER_UNSUPPORTED', path, 'must be an enumerable own data property');
    }
    if (!isAllowed(allowed, key)) {
      fail('SCHEDULER_UNSUPPORTED', path, 'is not a supported option');
    }
    record[key] = descriptor.value;
  }
  return record;
}

// Read an array length without a property get. Array `length` is a data
// property; a Proxy trap can still run, but an ordinary array is not read
// through its prototype or through a caller-defined getter.
function arrayLength(value, field) {
  const descriptor = getOwnPropertyDescriptor(value, 'length');
  if (
    descriptor === undefined ||
    hasOwn(descriptor, 'get') ||
    hasOwn(descriptor, 'set') ||
    typeof descriptor.value !== 'number'
  ) {
    fail('SCHEDULER_TYPE', field, 'must be an array');
  }
  return descriptor.value;
}

// Read an array element without invoking an accessor. Arrays are values, not
// configuration records, but an accessor index would be a side effect just the
// same. A hole is returned as undefined and fails the caller's type check.
function arrayIndexValue(value, index, field) {
  const descriptor = getOwnPropertyDescriptor(value, index + '');
  if (descriptor === undefined) {
    return undefined;
  }
  if (hasOwn(descriptor, 'get') || hasOwn(descriptor, 'set')) {
    fail('SCHEDULER_UNSUPPORTED', field, 'must be an enumerable data property, not an accessor');
  }
  if (!descriptor.enumerable) {
    fail('SCHEDULER_UNSUPPORTED', field, 'must be an enumerable own data property');
  }
  return descriptor.value;
}

function checkText(value, field, allowEmpty, max, allowWhitespaceEdges) {
  if (typeof value !== 'string') {
    fail('SCHEDULER_TYPE', field, 'must be a string');
  }
  if (value.length === 0 && !allowEmpty) {
    fail('SCHEDULER_VALUE', field, 'must not be empty');
  }
  if (value.length > max) {
    fail('SCHEDULER_LIMIT', field, 'is too long');
  }
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const low = value.charCodeAt(index + 1);
      if (!(low >= 0xdc00 && low <= 0xdfff)) {
        fail('SCHEDULER_VALUE', field, 'contains an unpaired surrogate');
      }
      index += 1;
      continue;
    }
    if (unit >= 0xdc00 && unit <= 0xdfff) {
      fail('SCHEDULER_VALUE', field, 'contains an unpaired surrogate');
    }
    if (unit <= 0x1f || (unit >= 0x7f && unit <= 0x9f)) {
      fail('SCHEDULER_VALUE', field, 'contains a control character');
    }
    if (unit === 0xfffe || unit === 0xffff) {
      fail('SCHEDULER_VALUE', field, 'contains an unsupported character');
    }
  }
  if (!allowWhitespaceEdges && trim(value) !== value) {
    fail('SCHEDULER_VALUE', field, 'must not have leading or trailing whitespace');
  }
  return value;
}

function requiredText(options, key, field, allowEmpty, max, allowWhitespaceEdges) {
  const value = options[key];
  if (value === undefined) {
    fail('SCHEDULER_VALUE', field, 'is required');
  }
  return checkText(value, field, allowEmpty, max, allowWhitespaceEdges);
}

function optionalText(options, key, field, allowEmpty, max, allowWhitespaceEdges) {
  const value = options[key];
  if (value === undefined) {
    return undefined;
  }
  return checkText(value, field, allowEmpty, max, allowWhitespaceEdges);
}

function checkBoolean(options, key, field) {
  const value = options[key];
  if (value === undefined) {
    return false;
  }
  if (typeof value !== 'boolean') {
    fail('SCHEDULER_TYPE', field, 'must be a boolean');
  }
  return value;
}

function checkName(value, field, max, systemd) {
  if (value === undefined) {
    fail('SCHEDULER_VALUE', field, 'is required');
  }
  const text = checkText(value, field, false, max, false);
  if (systemd) {
    if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(text)) {
      fail(
        'SCHEDULER_UNSUPPORTED',
        field,
        'must contain only ASCII letters, digits, underscores, dots, and hyphens, and must start with a letter or digit',
      );
    }
    if (text.endsWith('.') || text.endsWith('.service') || text.endsWith('.timer')) {
      fail('SCHEDULER_UNSUPPORTED', field, 'must not end with a dot or a unit suffix');
    }
  } else {
    if (text === '.' || text === '..') {
      fail('SCHEDULER_VALUE', field, 'must be a task name, not a directory');
    }
    if (text.includes('\\') || text.includes('/')) {
      fail('SCHEDULER_UNSUPPORTED', field, 'must not contain a path separator');
    }
  }
  return text;
}

function checkSystemdDescription(value) {
  const text = checkText(value, 'description', false, MAX_DESCRIPTION_LENGTH, false);
  if (/[\\%"']/.test(text)) {
    fail(
      'SCHEDULER_UNSUPPORTED',
      'description',
      'must not contain a backslash, percent sign, or quotation mark',
    );
  }
  return text;
}

function checkSystemdWorkingDirectory(value) {
  const text = checkText(value, 'workingDirectory', false, MAX_POSIX_PATH_LENGTH, false);
  if (text.charCodeAt(0) !== 0x2f) {
    fail('SCHEDULER_VALUE', 'workingDirectory', 'must be an absolute POSIX path');
  }
  if (text.length > 1 && text.endsWith('/')) {
    fail('SCHEDULER_VALUE', 'workingDirectory', 'must not end with a slash');
  }
  if (/[\\%"']/.test(text)) {
    fail(
      'SCHEDULER_UNSUPPORTED',
      'workingDirectory',
      'must not contain a backslash, percent sign, or quotation mark',
    );
  }
  return text;
}

function checkSystemdUser(value) {
  const text = checkText(value, 'user', false, 256, false);
  if (!/^(?:[0-9]+|[A-Za-z_][A-Za-z0-9_.-]*)$/.test(text)) {
    fail('SCHEDULER_UNSUPPORTED', 'user', 'must be a simple user name or a decimal numeric ID');
  }
  return text;
}

function checkSystemdProgram(value) {
  const text = checkText(value, 'program', false, MAX_ARGUMENT_LENGTH, true);
  if (text.charCodeAt(0) !== 0x2f) {
    fail('SCHEDULER_VALUE', 'program', 'must be an absolute POSIX path');
  }
  if (text === '/' || text.endsWith('/')) {
    fail('SCHEDULER_VALUE', 'program', 'must name an executable, not a directory');
  }
  if (text.includes('$')) {
    fail(
      'SCHEDULER_UNSUPPORTED',
      'program',
      'must not contain a dollar sign; the first ExecStart item may not be a variable',
    );
  }
  return text;
}

function checkArgs(value, field, allowEmpty, max) {
  if (value === undefined) {
    return [];
  }
  if (!isArray(value)) {
    fail('SCHEDULER_TYPE', field, 'must be an array');
  }
  const length = arrayLength(value, field);
  if (!isInteger(length) || length < 0) {
    fail('SCHEDULER_TYPE', field, 'must be an array');
  }
  if (length > MAX_ARGUMENTS) {
    fail('SCHEDULER_LIMIT', field, 'contains too many entries');
  }
  const args = [];
  for (let index = 0; index < length; index += 1) {
    const itemField = field + '[' + index + ']';
    const item = checkText(
      arrayIndexValue(value, index, itemField),
      itemField,
      allowEmpty,
      max,
      true,
    );
    args.push(item);
  }
  return args;
}

function checkSystemdArgs(value) {
  const args = checkArgs(value, 'args', true, MAX_ARGUMENT_LENGTH);
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === ';') {
      fail(
        'SCHEDULER_UNSUPPORTED',
        'args[' + index + ']',
        'a lone semicolon argument is ambiguous in ExecStart',
      );
    }
  }
  return args;
}

function quoteSystemdWord(word) {
  let result = '"';
  for (let index = 0; index < word.length; index += 1) {
    const character = word[index];
    if (character === '\\') {
      result += '\\\\';
    } else if (character === '"') {
      result += '\\"';
    } else if (character === '%') {
      result += '%%';
    } else if (character === '$') {
      result += '$$';
    } else {
      result += character;
    }
  }
  return result + '"';
}

function utf8Length(text) {
  let length = 0;
  for (let index = 0; index < text.length; index += 1) {
    const unit = text.charCodeAt(index);
    if (unit < 0x80) {
      length += 1;
    } else if (unit < 0x800) {
      length += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff) {
      length += 4;
      index += 1;
    } else {
      length += 3;
    }
  }
  return length;
}

function buildExecStart(program, args) {
  const words = [quoteSystemdWord(program)];
  for (let index = 0; index < args.length; index += 1) {
    words.push(quoteSystemdWord(args[index]));
  }
  const line = 'ExecStart=' + words.join(' ');
  if (utf8Length(line) > MAX_SYSTEMD_LINE_BYTES) {
    fail('SCHEDULER_LIMIT', 'ExecStart', 'line exceeds the systemd line-length limit');
  }
  return line;
}

function isAsciiDigit(character) {
  return character >= '0' && character <= '9';
}

function checkSchedule(value) {
  const schedule = validateRecord(value, 'schedule', SCHEDULE_KEYS);
  const kind = schedule.kind;
  if (kind !== 'daily' && kind !== 'weekly') {
    fail('SCHEDULER_SCHEDULE', 'schedule.kind', 'must be "daily" or "weekly"');
  }
  const at = schedule.at;
  if (typeof at !== 'string') {
    fail('SCHEDULER_TYPE', 'schedule.at', 'must be a string');
  }
  if (
    at.length !== 5 ||
    at[2] !== ':' ||
    !isAsciiDigit(at[0]) ||
    !isAsciiDigit(at[1]) ||
    !isAsciiDigit(at[3]) ||
    !isAsciiDigit(at[4])
  ) {
    fail('SCHEDULER_SCHEDULE', 'schedule.at', 'must be exactly HH:MM');
  }
  const hours = (at.charCodeAt(0) - 48) * 10 + (at.charCodeAt(1) - 48);
  const minutes = (at.charCodeAt(3) - 48) * 10 + (at.charCodeAt(4) - 48);
  if (hours > 23 || minutes > 59) {
    fail('SCHEDULER_SCHEDULE', 'schedule.at', 'must be a valid 24-hour time');
  }

  if (kind === 'daily') {
    if (schedule.weekdays !== undefined) {
      fail('SCHEDULER_SCHEDULE', 'schedule.weekdays', 'is only valid for a weekly schedule');
    }
    return { kind: kind, at: at, weekdays: [] };
  }

  const weekdays = schedule.weekdays;
  if (!isArray(weekdays)) {
    fail('SCHEDULER_TYPE', 'schedule.weekdays', 'must be an array');
  }
  const length = arrayLength(weekdays, 'schedule.weekdays');
  if (!isInteger(length) || length < 0) {
    fail('SCHEDULER_TYPE', 'schedule.weekdays', 'must be an array');
  }
  if (length === 0) {
    fail('SCHEDULER_VALUE', 'schedule.weekdays', 'must not be empty');
  }
  if (length > WEEKDAYS.length) {
    fail('SCHEDULER_VALUE', 'schedule.weekdays', 'contains too many entries');
  }
  const seen = create(null);
  for (let index = 0; index < length; index += 1) {
    const field = 'schedule.weekdays[' + index + ']';
    const day = arrayIndexValue(weekdays, index, field);
    if (typeof day !== 'string') {
      fail('SCHEDULER_TYPE', field, 'must be a string');
    }
    if (!isAllowed(WEEKDAYS, day)) {
      fail(
        'SCHEDULER_VALUE',
        field,
        'must be one of mon, tue, wed, thu, fri, sat, sun',
      );
    }
    if (seen[day]) {
      fail('SCHEDULER_VALUE', field, 'is duplicated');
    }
    seen[day] = true;
  }
  const sorted = [];
  for (let index = 0; index < WEEKDAYS.length; index += 1) {
    if (seen[WEEKDAYS[index]]) {
      sorted.push(WEEKDAYS[index]);
    }
  }
  return { kind: kind, at: at, weekdays: sorted };
}

function calendarExpression(schedule) {
  const time = schedule.at + ':00';
  if (schedule.kind === 'daily') {
    return '*-*-* ' + time;
  }
  let days = '';
  for (let index = 0; index < schedule.weekdays.length; index += 1) {
    if (index > 0) {
      days += ',';
    }
    days += SYSTEMD_WEEKDAY[schedule.weekdays[index]];
  }
  return days + ' *-*-* ' + time;
}

function daysInMonth(year, month) {
  if (month === 2) {
    const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
    return leap ? 29 : 28;
  }
  if (month === 4 || month === 6 || month === 9 || month === 11) {
    return 30;
  }
  return 31;
}

function checkDate(value) {
  if (typeof value !== 'string') {
    fail('SCHEDULER_TYPE', 'startDate', 'must be a string');
  }
  if (
    value.length !== 10 ||
    value[4] !== '-' ||
    value[7] !== '-' ||
    !isAsciiDigit(value[0]) ||
    !isAsciiDigit(value[1]) ||
    !isAsciiDigit(value[2]) ||
    !isAsciiDigit(value[3]) ||
    !isAsciiDigit(value[5]) ||
    !isAsciiDigit(value[6]) ||
    !isAsciiDigit(value[8]) ||
    !isAsciiDigit(value[9])
  ) {
    fail('SCHEDULER_VALUE', 'startDate', 'must be exactly YYYY-MM-DD');
  }
  const year =
    (value.charCodeAt(0) - 48) * 1000 +
    (value.charCodeAt(1) - 48) * 100 +
    (value.charCodeAt(2) - 48) * 10 +
    (value.charCodeAt(3) - 48);
  const month = (value.charCodeAt(5) - 48) * 10 + (value.charCodeAt(6) - 48);
  const day = (value.charCodeAt(8) - 48) * 10 + (value.charCodeAt(9) - 48);
  if (year < 1601 || year > 9999 || month < 1 || month > 12) {
    fail('SCHEDULER_VALUE', 'startDate', 'must be a Gregorian date from 1601 through 9999');
  }
  if (day < 1 || day > daysInMonth(year, month)) {
    fail('SCHEDULER_VALUE', 'startDate', 'is not a valid day in that month');
  }
  return value;
}

function isAsciiLetter(unit) {
  return (unit >= 0x41 && unit <= 0x5a) || (unit >= 0x61 && unit <= 0x7a);
}

function checkWindowsPath(value, field, allowRoot) {
  const text = checkText(value, field, false, MAX_WINDOWS_PATH_LENGTH, false);
  if (text.includes('/')) {
    fail('SCHEDULER_VALUE', field, 'must use backslash separators');
  }
  if (/["<>|?*]/.test(text)) {
    fail('SCHEDULER_UNSUPPORTED', field, 'contains a character Windows does not allow in a path');
  }
  if (text.includes('%')) {
    fail('SCHEDULER_UNSUPPORTED', field, 'must not contain a percent sign');
  }

  let components;
  if (text.startsWith('\\\\')) {
    if (text.startsWith('\\\\?\\') || text.startsWith('\\\\.\\')) {
      fail('SCHEDULER_UNSUPPORTED', field, 'device paths are not supported');
    }
    const parts = text.slice(2).split('\\');
    if (parts.length < 2 || parts[0] === '' || parts[1] === '') {
      fail('SCHEDULER_VALUE', field, 'must be an absolute UNC path with a server and share');
    }
    if (parts[0].includes(':')) {
      fail('SCHEDULER_VALUE', field, 'must not contain a colon in a server name');
    }
    if (parts[1].includes(':')) {
      fail('SCHEDULER_VALUE', field, 'must not contain a colon in a share name');
    }
    if (trim(parts[0]) !== parts[0] || trim(parts[1]) !== parts[1]) {
      fail('SCHEDULER_VALUE', field, 'has a server or share name with ambiguous whitespace');
    }
    components = [];
    for (let index = 2; index < parts.length; index += 1) {
      components.push(parts[index]);
    }
  } else {
    if (
      text.length < 3 ||
      !isAsciiLetter(text.charCodeAt(0)) ||
      text[1] !== ':' ||
      text[2] !== '\\'
    ) {
      fail('SCHEDULER_VALUE', field, 'must be an absolute drive path or a UNC path');
    }
    const rest = text.slice(3);
    components = rest === '' ? [] : rest.split('\\');
  }

  let trailingSeparator = false;
  if (components.length > 0 && components[components.length - 1] === '') {
    trailingSeparator = true;
    components.pop();
  }
  if (trailingSeparator && !allowRoot) {
    fail('SCHEDULER_VALUE', field, 'must not end with a backslash');
  }
  if (components.length === 0) {
    if (!allowRoot) {
      fail('SCHEDULER_VALUE', field, 'must name an executable, not a directory');
    }
    return text;
  }
  for (let index = 0; index < components.length; index += 1) {
    const component = components[index];
    if (component === '') {
      fail('SCHEDULER_VALUE', field, 'must not contain an empty path component');
    }
    if (component === '.' || component === '..') {
      fail('SCHEDULER_VALUE', field, 'must not contain a dot or dot-dot path component');
    }
    if (component.includes(':')) {
      fail('SCHEDULER_VALUE', field, 'must not contain a colon outside the drive prefix');
    }
    if (trim(component) !== component || component.endsWith('.')) {
      fail('SCHEDULER_VALUE', field, 'has a path component with ambiguous leading, trailing, or dot whitespace');
    }
  }
  return text;
}

function quoteWindowsArgument(argument) {
  if (argument.length === 0) {
    return '""';
  }
  let needsQuotes = false;
  for (let index = 0; index < argument.length; index += 1) {
    const character = argument[index];
    if (character === ' ' || character === '\t' || character === '"') {
      needsQuotes = true;
      break;
    }
  }
  if (!needsQuotes) {
    return argument;
  }
  let result = '"';
  let backslashes = 0;
  for (let index = 0; index < argument.length; index += 1) {
    const character = argument[index];
    if (character === '\\') {
      backslashes += 1;
      continue;
    }
    if (character === '"') {
      for (let count = 0; count < backslashes * 2 + 1; count += 1) {
        result += '\\';
      }
      result += '"';
      backslashes = 0;
    } else {
      for (let count = 0; count < backslashes; count += 1) {
        result += '\\';
      }
      result += character;
      backslashes = 0;
    }
  }
  for (let count = 0; count < backslashes * 2; count += 1) {
    result += '\\';
  }
  return result + '"';
}

function windowsArguments(args) {
  let text = '';
  for (let index = 0; index < args.length; index += 1) {
    if (index > 0) {
      text += ' ';
    }
    text += quoteWindowsArgument(args[index]);
  }
  return text;
}

function escapeXml(value) {
  let result = '';
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (character === '&') {
      result += '&amp;';
    } else if (character === '<') {
      result += '&lt;';
    } else if (character === '>') {
      result += '&gt;';
    } else if (character === '"') {
      result += '&quot;';
    } else if (character === "'") {
      result += '&apos;';
    } else {
      result += character;
    }
  }
  return result;
}

function checkPrincipal(value) {
  if (value === undefined) {
    return undefined;
  }
  const principal = validateRecord(value, 'principal', PRINCIPAL_KEYS);
  const userId = requiredText(
    principal,
    'userId',
    'principal.userId',
    false,
    MAX_USER_ID_LENGTH,
    false,
  );
  const logonType = requiredText(
    principal,
    'logonType',
    'principal.logonType',
    false,
    64,
    false,
  );
  if (!hasOwn(LOGON_TYPES, logonType)) {
    fail(
      'SCHEDULER_VALUE',
      'principal.logonType',
      'must be InteractiveToken, Password, S4U, or InteractiveTokenOrPassword',
    );
  }
  const runLevel = optionalText(
    principal,
    'runLevel',
    'principal.runLevel',
    false,
    64,
    false,
  );
  if (runLevel !== undefined && !hasOwn(RUN_LEVELS, runLevel)) {
    fail('SCHEDULER_VALUE', 'principal.runLevel', 'must be LeastPrivilege or HighestAvailable');
  }
  return { userId: userId, logonType: logonType, runLevel: runLevel };
}

function buildServiceUnchecked(options) {
  checkName(options.name, 'name', MAX_UNIT_NAME_LENGTH, true);
  const description = optionalText(
    options,
    'description',
    'description',
    false,
    MAX_DESCRIPTION_LENGTH,
    false,
  );
  if (description !== undefined) {
    checkSystemdDescription(description);
  }
  const program = checkSystemdProgram(options.program);
  const args = checkSystemdArgs(options.args);
  const workingDirectory = optionalText(
    options,
    'workingDirectory',
    'workingDirectory',
    false,
    MAX_POSIX_PATH_LENGTH,
    false,
  );
  if (workingDirectory !== undefined) {
    checkSystemdWorkingDirectory(workingDirectory);
  }
  const user = optionalText(options, 'user', 'user', false, 256, false);
  if (user !== undefined) {
    checkSystemdUser(user);
  }

  const lines = [];
  if (description !== undefined) {
    lines.push('[Unit]');
    lines.push('Description=' + description);
    lines.push('');
  }
  lines.push('[Service]');
  lines.push('Type=oneshot');
  lines.push(buildExecStart(program, args));
  if (user !== undefined) {
    lines.push('User=' + user);
  }
  if (workingDirectory !== undefined) {
    lines.push('WorkingDirectory=' + workingDirectory);
  }
  return lines.join('\n') + '\n';
}

function buildTimerUnchecked(options) {
  const name = checkName(options.name, 'name', MAX_UNIT_NAME_LENGTH, true);
  const schedule = checkSchedule(options.schedule);
  const persistent = checkBoolean(options, 'persistent', 'persistent');
  const lines = [];
  lines.push('[Timer]');
  lines.push('OnCalendar=' + calendarExpression(schedule));
  if (persistent) {
    lines.push('Persistent=true');
  }
  lines.push('Unit=' + name + '.service');
  lines.push('');
  lines.push('[Install]');
  lines.push('WantedBy=timers.target');
  return lines.join('\n') + '\n';
}

function buildWindowsTask(options) {
  const name = checkName(options.name, 'name', MAX_TASK_NAME_LENGTH, false);
  const description = optionalText(
    options,
    'description',
    'description',
    false,
    MAX_DESCRIPTION_LENGTH,
    true,
  );
  const startDate = checkDate(options.startDate);
  const program = checkWindowsPath(options.program, 'program', false);
  const args = checkArgs(options.args, 'args', true, MAX_ARGUMENT_LENGTH);
  const workingDirectory = optionalText(
    options,
    'workingDirectory',
    'workingDirectory',
    false,
    MAX_WINDOWS_PATH_LENGTH,
    false,
  );
  if (workingDirectory !== undefined) {
    checkWindowsPath(workingDirectory, 'workingDirectory', true);
  }
  const schedule = checkSchedule(options.schedule);
  const principal = checkPrincipal(options.principal);
  const argumentsText = windowsArguments(args);
  const commandLineLength = program.length + (args.length === 0 ? 0 : 1 + argumentsText.length);
  if (commandLineLength > MAX_WINDOWS_COMMAND_LINE - 1) {
    fail('SCHEDULER_LIMIT', 'commandLine', 'exceeds the Windows command-line length limit');
  }

  const lines = [];
  lines.push('<?xml version="1.0" encoding="UTF-16"?>');
  // The authoritative schema fixes Task/@version to 1.3. The 1.2 value used
  // by older examples fails genuine XSD validation.
  lines.push(
    '<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">',
  );
  lines.push('  <RegistrationInfo>');
  if (description !== undefined) {
    lines.push('    <Description>' + escapeXml(description) + '</Description>');
  }
  lines.push('    <URI>\\' + escapeXml(name) + '</URI>');
  lines.push('  </RegistrationInfo>');
  lines.push('  <Triggers>');
  lines.push('    <CalendarTrigger>');
  // triggerBaseType is an xs:sequence: Enabled precedes StartBoundary.
  lines.push('      <Enabled>true</Enabled>');
  lines.push('      <StartBoundary>' + startDate + 'T' + schedule.at + ':00</StartBoundary>');
  if (schedule.kind === 'daily') {
    lines.push('      <ScheduleByDay>');
    lines.push('        <DaysInterval>1</DaysInterval>');
    lines.push('      </ScheduleByDay>');
  } else {
    // weeklyScheduleType is an xs:all, so either order is schema-valid. Emit
    // DaysOfWeek first as the canonical order this API documents.
    lines.push('      <ScheduleByWeek>');
    lines.push('        <DaysOfWeek>');
    for (let index = 0; index < schedule.weekdays.length; index += 1) {
      lines.push('          <' + WINDOWS_WEEKDAY[schedule.weekdays[index]] + ' />');
    }
    lines.push('        </DaysOfWeek>');
    lines.push('        <WeeksInterval>1</WeeksInterval>');
    lines.push('      </ScheduleByWeek>');
  }
  lines.push('    </CalendarTrigger>');
  lines.push('  </Triggers>');
  if (principal !== undefined) {
    lines.push('  <Principals>');
    lines.push('    <Principal id="Author">');
    lines.push('      <UserId>' + escapeXml(principal.userId) + '</UserId>');
    lines.push('      <LogonType>' + principal.logonType + '</LogonType>');
    if (principal.runLevel !== undefined) {
      lines.push('      <RunLevel>' + principal.runLevel + '</RunLevel>');
    }
    lines.push('    </Principal>');
    lines.push('  </Principals>');
  }
  // Actions/@Context is an xs:IDREF into the Principal ids. Emitting it when
  // <Principals> is omitted fails the schema's ContextKeyRef keyref.
  lines.push(principal === undefined ? '  <Actions>' : '  <Actions Context="Author">');
  lines.push('    <Exec>');
  lines.push('      <Command>' + escapeXml(program) + '</Command>');
  if (args.length > 0) {
    lines.push('      <Arguments>' + escapeXml(argumentsText) + '</Arguments>');
  }
  if (workingDirectory !== undefined) {
    lines.push('      <WorkingDirectory>' + escapeXml(workingDirectory) + '</WorkingDirectory>');
  }
  lines.push('    </Exec>');
  lines.push('  </Actions>');
  lines.push('</Task>');
  return lines.join('\n') + '\n';
}

function systemdService(options) {
  return buildServiceUnchecked(validateRecord(options, '', SYSTEMD_SERVICE_KEYS));
}

function systemdTimer(options) {
  return buildTimerUnchecked(validateRecord(options, '', SYSTEMD_TIMER_KEYS));
}

function systemdUnits(options) {
  const record = validateRecord(options, '', SYSTEMD_UNITS_KEYS);
  return { service: buildServiceUnchecked(record), timer: buildTimerUnchecked(record) };
}

function windowsTask(options) {
  return buildWindowsTask(validateRecord(options, '', WINDOWS_TASK_KEYS));
}

export {
  SchedulerError,
  WINDOWS_XML_ENCODING,
  MAX_SYSTEMD_LINE_BYTES,
  MAX_WINDOWS_COMMAND_LINE,
  systemdService,
  systemdTimer,
  systemdUnits,
  windowsTask,
};
