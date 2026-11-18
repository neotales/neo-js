import { stdout } from "neo:process";

// This is a compile-only fixture. An unused directive fails the check.
// @ts-expect-error stdout.write accepts strings or Uint8Arrays, not numbers.
stdout.write(42);

// @ts-expect-error os.version is optional and cannot always be used as a string.
os.version.toUpperCase();
