/// <reference path="./neo-js.d.ts" />

import * as registry from "neo:win-registry";
import * as legacy from "neo:winregistry";

const canonicalExports: typeof registry = legacy;
const legacyExports: typeof legacy = registry;

declare const canonicalValue: registry.RawValue;
declare const legacyValue: legacy.RawValue;
const canonicalRaw: registry.RawValue = legacyValue;
const legacyRaw: legacy.RawValue = canonicalValue;

declare const lookalike: Pick<registry.RegistryKey, keyof registry.RegistryKey>;
// @ts-expect-error Matching the public shape does not create a branded key.
const forged: registry.RegistryKey = lookalike;

function ownedKeyTypes(path: string): void {
  using key: registry.RegistryKey = registry.Registry.createKey(
    path,
    registry.Rights.QUERY_VALUE | registry.Rights.SET_VALUE,
  );
  const keyPath: string = key.path;
  const created: boolean = key.created;
  key.setString("", key.getString(""));
  key[Symbol.dispose]();
  key.close();

  using legacyKey: legacy.RegistryKey = legacy.Registry.openKey(path);
  const sameBrand: registry.RegistryKey = legacyKey;
  // @ts-expect-error An owned key cannot be constructed by the caller.
  new registry.RegistryKey();
  // @ts-expect-error The native handle is never exposed.
  legacyKey.unwrap();
  // @ts-expect-error Access masks are numbers, not raw handles.
  registry.Registry.openKey(path, 1n);
}
