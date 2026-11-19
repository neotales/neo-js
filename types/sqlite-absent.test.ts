/// <reference path="./neo-js.core.d.ts" />

// Negative fixture: a host built without the optional neojs-sqlite crate must
// reject this import. examples/typed-script/smoke.sh requires the type checker
// to fail here, and to fail for this module rather than for another reason.
import { openMemory } from "neo:sqlite";

export const open = openMemory;
