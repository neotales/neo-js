import process, { ElevationError, isElevated, stdout } from "neo:process";
import { readFile, readFileSync } from "neo:fs";

const elevated: boolean = isElevated();
const alsoElevated: boolean = process.isElevated();
const elevationError: typeof ElevationError = process.ElevationError;

// This is a compile-only fixture. An unused directive fails the check.
// @ts-expect-error isElevated is synchronous and returns a boolean.
const pendingElevation: Promise<boolean> = isElevated();

// @ts-expect-error stdout.write accepts strings or Uint8Arrays, not numbers.
stdout.write(42);

// @ts-expect-error os.version is optional and cannot always be used as a string.
os.version.toUpperCase();

const pending: Promise<Uint8Array> = readFile("data.bin");
const bytes: Uint8Array = readFileSync("data.bin");
const globalPending: Promise<Uint8Array> = fs.readFile("data.bin");
const globalBytes: Uint8Array = fs.readFileSync("data.bin");

// @ts-expect-error The unsuffixed method no longer returns bytes synchronously.
const notBytes: Uint8Array = readFile("data.bin");
