import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import process from "node:process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const cache = fileURLToPath(new URL("../node_modules/.cache/", import.meta.url));

/** Copy the inputs so a mutation cannot affect the checkout or the generated pages. */
async function fixture(t) {
	await mkdir(cache, { recursive: true });
	const directory = await mkdtemp(join(cache, "reference-fixture-"));
	t.after(() => rm(directory, { recursive: true, force: true }));
	for (const folder of ["types", "crates/neo-js/src", "docs/site/scripts"]) {
		await mkdir(join(directory, folder), { recursive: true });
	}
	for (const file of [
		"types/neo-js.d.ts",
		"crates/neo-js/src/modules.rs",
		"docs/site/scripts/reference.mjs",
		"docs/site/scripts/runtime-inventory.mjs",
	]) {
		await copyFile(join(root, file), join(directory, file));
	}
	return {
		api: join(directory, "docs/site/src/content/docs/api"),
		types: join(directory, "types/neo-js.d.ts"),
		run(script, ...args) {
			const result = spawnSync(
				process.execPath,
				[join(directory, "docs/site/scripts", script), ...args],
				{
					encoding: "utf8",
					// Pass only the inputs these scripts need.
					env: {
						NEO_JS_DOCS_BIN: process.env.NEO_JS_DOCS_BIN ?? "",
						TMPDIR: directory,
						TEMP: directory,
						TMP: directory,
					},
					timeout: 30_000,
				},
			);
			if (result.error) throw result.error;
			return { status: result.status, output: result.stdout + result.stderr };
		},
	};
}

/** Assert both success and the specific error when a mutation should fail. */
function expect(result, status, message) {
	assert.equal(result.status, status, result.output);
	assert.match(result.output, message);
}

test("reference rejects every extra publishable extension and nested directory", async (t) => {
	const { api, run } = await fixture(t);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(run("reference.mjs", "--check"), 0, /API reference matches/);

	const extras = ["mdx", "markdown", "mdown", "mkdn", "mkd", "mdwn", "md"].map(
		(extension) => `old.${extension}`,
	);
	for (const name of extras) {
		await writeFile(join(api, name), "---\ntitle: Extra\n---\n");
	}
	await mkdir(join(api, "modules/archive"));
	await writeFile(join(api, "modules/archive/index.mdx"), "---\ntitle: Archive\n---\n");
	for (const args of [["--check"], []]) {
		const result = run("reference.mjs", ...args);
		expect(result, 1, /Unexpected entries in the generated API reference/);
		for (const name of [...extras, "modules/archive"]) assert.ok(result.output.includes(name));
	}
	assert.match(await readFile(join(api, "old.mdx"), "utf8"), /title: Extra/);
});

test("generation can remove a stale generated module page", async (t) => {
	const { api, run } = await fixture(t);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	await copyFile(join(api, "modules/hash.md"), join(api, "modules/obsolete.md"));
	expect(run("reference.mjs", "--check"), 1, /Unexpected entries .+: modules\/obsolete\.md/);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(run("reference.mjs", "--check"), 0, /API reference matches/);
});

test("reference rejects a removed or added declaration even after regeneration", async (t) => {
	const { types, run } = await fixture(t);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(run("runtime-inventory.mjs"), 0, /Runtime value exports match/);

	const source = await readFile(types, "utf8");
	const added = source.replace(
		'declare module "neo:hash" {',
		'declare module "neo:hash" {\n  export const reviewerExtra: number;',
	);
	assert.notEqual(added, source);
	await writeFile(types, added);
	expect(run("reference.mjs", "--check"), 1, /API reference differs: modules\/hash\.md/);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(run("runtime-inventory.mjs"), 1, /declared but missing at runtime: reviewerExtra/);

	const removed = source.replace(/  export function digestFile\([^;\n]+;\n/, "");
	assert.notEqual(removed, source);
	await writeFile(types, removed);
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(
		run("runtime-inventory.mjs"),
		1,
		/runtime exports absent from declarations or unexpected for this permission: digestFile/,
	);
});

test("inventory counts default values but excludes type-only aliases and wildcard re-exports", async (t) => {
	const { types, run } = await fixture(t);
	const source = await readFile(types, "utf8");
	const added = source.replace(
		'declare module "neo:hash" {',
		`declare module "neo:hash" {
  export type { parse as ParseType } from "neo:dotenv";
  export type * from "neo:semver";`,
	);
	assert.notEqual(added, source);
	await writeFile(types, added);
	expect(run("runtime-inventory.mjs"), 0, /Runtime value exports match/);

	const changedDefault = added.replace(
		"export default parse;",
		"export type { parse as default };",
	);
	assert.notEqual(changedDefault, added);
	await writeFile(types, changedDefault);
	expect(
		run("runtime-inventory.mjs"),
		1,
		/neo:dotenv .+: runtime exports absent from declarations or unexpected for this permission: default/,
	);
});
