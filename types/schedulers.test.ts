/// <reference path="./neo-js.d.ts" />

import {
  MAX_SYSTEMD_LINE_BYTES,
  type Schedule,
  SchedulerError as SystemdError,
  type SystemdUnitsOptions,
  systemdUnits,
} from "neo:os/schedulers/systemd";
import {
  MAX_WINDOWS_COMMAND_LINE,
  type Schedule as WindowsSchedule,
  SchedulerError as WindowsError,
  WINDOWS_XML_ENCODING,
  type WindowsTaskOptions,
  windowsTask,
} from "neo:os/schedulers/windows";
import * as legacy from "neo:scheduler";

const schedule: Schedule = { kind: "weekly", at: "09:30", weekdays: ["mon", "wed"] };
const windowsSchedule: WindowsSchedule = schedule;
const systemdOptions: SystemdUnitsOptions = { name: "backup", program: "/bin/true", schedule };
const windowsOptions: WindowsTaskOptions = {
  name: "Backup",
  startDate: "2026-11-18",
  program: "C:\\backup.exe",
  schedule: windowsSchedule,
};
const units: { service: string; timer: string } = systemdUnits(systemdOptions);
const task: string = windowsTask(windowsOptions);
const unitLimit: 1048576 = MAX_SYSTEMD_LINE_BYTES;
const taskLimit: 32767 = MAX_WINDOWS_COMMAND_LINE;
const encoding: "UTF-16LE" = WINDOWS_XML_ENCODING;

declare const error: unknown;
if (error instanceof SystemdError || error instanceof WindowsError) {
  const code: legacy.SchedulerErrorCode = error.code;
  const field: string = error.field;
}
legacy.systemdUnits(systemdOptions);
legacy.windowsTask(windowsOptions);

// @ts-expect-error The systemd entry point has no Windows generator.
import { windowsTask as missingWindowsTask } from "neo:os/schedulers/systemd";
// @ts-expect-error The Windows entry point has no systemd generator.
import { systemdUnits as missingSystemdUnits } from "neo:os/schedulers/windows";
// @ts-expect-error There is no combined canonical parent module.
import { windowsTask as missingParent } from "neo:os/schedulers";
