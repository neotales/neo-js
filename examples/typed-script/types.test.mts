import process, { ElevationError, isElevated, stdout } from "neo:process";

const elevated: boolean = isElevated();
const alsoElevated: boolean = process.isElevated();
const elevationError: typeof ElevationError = process.ElevationError;

// This is a compile-only fixture. An unused directive fails the check.
// @ts-expect-error isElevated is synchronous and returns a boolean.
const pending: Promise<boolean> = isElevated();

// @ts-expect-error stdout.write accepts strings or Uint8Arrays, not numbers.
stdout.write(42);

// @ts-expect-error os.version is optional and cannot always be used as a string.
os.version.toUpperCase();
