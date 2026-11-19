import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFile, cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import process from "node:process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const cache = fileURLToPath(new URL("../node_modules/.cache/", import.meta.url));

/** Copy the inputs so a mutation cannot affect the checkout or the generated pages. */
async function fixture(t, runtime = false) {
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
	if (runtime) {
		for (const path of ["Cargo.toml", "Cargo.lock", "crates"]) {
			await cp(join(root, path), join(directory, path), { recursive: true });
		}
	}
	return {
		api: join(directory, "docs/site/src/content/docs/api"),
		types: join(directory, "types/neo-js.d.ts"),
		checkout: directory,
		run(script, ...args) {
			const result = spawnSync(
				process.execPath,
				[join(directory, "docs/site/scripts", script), ...args],
				{
					encoding: "utf8",
					timeout: 300_000,
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
	const nested = "modules/os/schedulers/old.mdx";
	await writeFile(join(api, nested), "---\ntitle: Old scheduler\n---\n");
	for (const args of [["--check"], []]) {
		const result = run("reference.mjs", ...args);
		expect(result, 1, /Unexpected entries in the generated API reference/);
		for (const name of [...extras, "modules/archive", nested])
			assert.ok(result.output.includes(name));
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

/** Change declarations without touching the live checkout or cached runtime. */
async function checkDeclarationDrift({ types, run }, source) {
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
}

/** Verify that a declared type-only export cannot be mistaken for a value. */
async function checkTypeOnlyExports({ types, run }, source) {
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
}

test("runtime inventory refreshes its checkout's CLI and checks for export and flag drift", async (t) => {
	const inputs = await fixture(t, true);
	const { checkout, types, run } = inputs;
	const declarations = await readFile(types, "utf8");
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
	expect(run("runtime-inventory.mjs"), 0, /Runtime value exports match/);

	await t.test("rejects missing or extra value declarations after regenerating pages", async () => {
		await checkDeclarationDrift(inputs, declarations);
	});
	await t.test(
		"counts default values but ignores type-only aliases and wildcard re-exports",
		async () => {
			await checkTypeOnlyExports(inputs, declarations);
			await writeFile(types, declarations);
			expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);
		},
	);

	const module = join(checkout, "crates/neo-js/src/js/string_builder.js");
	const original = await readFile(module, "utf8");
	await t.test("rebuilds a stale CLI and detects an extra runtime export", async () => {
		await writeFile(module, `${original}\nexport const reviewerExtra = 1;\n`);
		expect(
			run("runtime-inventory.mjs"),
			1,
			/runtime exports absent from declarations or unexpected for this permission: reviewerExtra/,
		);
	});
	await t.test("rebuilds after a runtime export is removed", async () => {
		const missing = original.replace("export class StringBuilder", "class StringBuilder");
		assert.notEqual(missing, original);
		await writeFile(module, missing);
		expect(run("runtime-inventory.mjs"), 1, /declared but missing at runtime: StringBuilder/);
		await writeFile(module, original);
	});

	const cli = join(checkout, "crates/neo-js-cli/src/main.rs");
	const cliSource = await readFile(cli, "utf8");
	await t.test(
		"rejects a new permission flag and a removed flag from current CLI help",
		async () => {
			const renamed = cliSource.replace(
				"#[arg(long)]\n    allow_net: bool,",
				'#[arg(long = "allow-reviewer")]\n    allow_net: bool,',
			);
			assert.notEqual(renamed, cliSource);
			await writeFile(cli, renamed);
			expect(
				run("runtime-inventory.mjs"),
				1,
				/Unmodeled in check: --allow-reviewer\. Not listed in CLI help: --allow-net\./,
			);
		},
	);
	await t.test(
		"fails closed when Cargo cannot rebuild and an older executable is present",
		async () => {
			await writeFile(cli, `${cliSource}\ncompile_error!("regression fixture");\n`);
			expect(
				run("runtime-inventory.mjs"),
				1,
				/Could not build the current neo-js checkout offline/,
			);
			await writeFile(cli, cliSource);
			expect(run("runtime-inventory.mjs"), 0, /Runtime value exports match/);
		},
	);
});
