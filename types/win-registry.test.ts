/// <reference path="./neo-js.d.ts" />

import * as registry from "neo:win-registry";
import * as legacy from "neo:winregistry";

const canonicalExports: typeof registry = legacy;
const legacyExports: typeof legacy = registry;

declare const canonicalValue: registry.RawValue;
declare const legacyValue: legacy.RawValue;
const canonicalRaw: registry.RawValue = legacyValue;
const legacyRaw: legacy.RawValue = canonicalValue;
