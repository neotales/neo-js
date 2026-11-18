/// <reference path="./neo-js.d.ts" />

import { download, type DownloadOptions } from "neo:fetch";
import { doesNotThrow, throws } from "neo:assert";
import test, { test as namedTest } from "neo:test";

declare const url: string, path: string;
declare const condition: boolean;
const seen: Array<[number, number | null]> = [];

download(url, path);
download(url, path, { progress: (received, total) => seen.push([received, total]) });
download(url, path, { progress(received, total) { seen.push([received, total]); } });
test("no return", () => {});
namedTest("ignored number", () => 42);

// @ts-expect-error An inferred async progress callback is not supported.
download(url, path, { progress: async () => {} });
// @ts-expect-error An ordinary callback returning a Promise is not supported either.
download(url, path, { progress: () => Promise.reject(new Error()) });
// @ts-expect-error A union with a thenable is still not supported.
download(url, path, { progress: () => condition ? 0 : { then() {} } });
const inferredOptions = { progress: () => Promise.resolve() };
// @ts-expect-error Preserving the inferred options type preserves the rejection.
download(url, path, inferredOptions);
const checkedOptions = { progress: async () => {} } satisfies DownloadOptions;
// @ts-expect-error `satisfies` checks the options without erasing the return type.
download(url, path, checkedOptions);

// @ts-expect-error Async test.
test("async", async () => {});
// @ts-expect-error Synchronous function with a Promise on only one branch.
namedTest("sometimes a promise", () => condition ? 42 : Promise.resolve());
// @ts-expect-error Structural thenable.
test("thenable", () => ({ then() {} }));
// @ts-expect-error Generators are not supported.
test("generator", function* () { yield 1; });
// @ts-expect-error A callback that returns a generator is not supported either.
test("returns a generator", () => (function* () { yield 1; })());
// @ts-expect-error Async generators are not supported.
test("async generator", async function* () { yield 1; });

const returned: { ok: true } = doesNotThrow(() => ({ ok: true as const }));
const explicitlyTyped: number = doesNotThrow<number>(() => 42);
const caught: RangeError = throws(() => { throw new RangeError(); }, RangeError);
// @ts-expect-error Assertions reject inferred Promise returns.
doesNotThrow(() => Promise.resolve());
// @ts-expect-error A generator return on one branch is still unsupported.
doesNotThrow(() => condition ? 42 : (function* () {})());
// @ts-expect-error The constructor matcher does not make a thenable supported.
throws(() => ({ then() {} }), TypeError);
// @ts-expect-error Neither does a RegExp matcher.
throws(async () => {}, /message/);
// @ts-expect-error The matcher-free overload also rejects async generators.
throws(async function* () {});

// These compile: TypeScript has already erased the callback return type.
// The runtime still rejects them.
const erased: () => void = async () => {};
const erasedOptions: DownloadOptions = { progress: erased };
download(url, path, erasedOptions);
test("erased", erased);
doesNotThrow(erased);
throws(erased, TypeError);
