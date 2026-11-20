import assert from "node:assert/strict";
import { execFile, spawnSync } from "node:child_process";
import {
	copyFile,
	cp,
	mkdir,
	mkdtemp,
	readdir,
	readFile,
	rm,
	stat,
	writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import process from "node:process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const cache = fileURLToPath(new URL("../node_modules/.cache/", import.meta.url));
const execFileAsync = promisify(execFile);
const cliBinary = process.platform === "win32" ? "neo-js.exe" : "neo-js";

/** Allocate and remove only a fresh directory owned by this test. */
async function ownedDirectory(t) {
	await mkdir(cache, { recursive: true });
	const directory = await mkdtemp(join(cache, "reference-fixture-"));
	t.after(() => rm(directory, { recursive: true, force: true }));
	return directory;
}

/** Copy inputs and isolate both source mutations and Cargo output. */
async function fixture(t, runtime = false, inheritedEnv = process.env) {
	const directory = await ownedDirectory(t);
	const target = join(directory, "target");
	const options = {
		encoding: "utf8",
		timeout: 300_000,
		env: { ...inheritedEnv, CARGO_TARGET_DIR: target },
	};
	const argsFor = (script, args) => [join(directory, "docs/site/scripts", script), ...args];
	for (const folder of ["types", "crates/neo-js/src", "docs/site/scripts"]) {
		await mkdir(join(directory, folder), { recursive: true });
	}
	for (const file of [
		"types/neo-js.d.ts",
		"types/neo-js.core.d.ts",
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
		target,
		run(script, ...args) {
			const result = spawnSync(process.execPath, argsFor(script, args), options);
			if (result.error) throw result.error;
			return { status: result.status, output: result.stdout + result.stderr };
		},
		async runAsync(script, ...args) {
			try {
				const { stdout, stderr } = await execFileAsync(
					process.execPath,
					argsFor(script, args),
					options,
				);
				return { status: 0, output: stdout + stderr };
			} catch (error) {
				if (typeof error.code !== "number") throw error;
				return { status: error.code, output: error.stdout + error.stderr };
			}
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
	const shared = await ownedDirectory(t);
	const marker = join(shared, "sentinel");
	await writeFile(marker, "leave this target alone");
	const inheritedEnv = { ...process.env, CARGO_TARGET_DIR: shared };
	const inputs = await fixture(t, true, inheritedEnv);
	const { checkout, types, run } = inputs;
	const declarations = await readFile(types, "utf8");
	expect(run("reference.mjs"), 0, /Generated \d+ API reference pages/);

	await t.test(
		"concurrent builds get distinct binaries and ignore the inherited target",
		async (t) => {
			const sibling = await fixture(t, true, inheritedEnv);
			const module = join(sibling.checkout, "crates/neo-js/src/js/string_builder.js");
			const original = await readFile(module, "utf8");
			await writeFile(module, `${original}\nexport const isolationMarker = 1;\n`);
			const [normal, mutated] = await Promise.allSettled([
				inputs.runAsync("runtime-inventory.mjs"),
				sibling.runAsync("runtime-inventory.mjs"),
			]);
			if (normal.status === "rejected") throw normal.reason;
			if (mutated.status === "rejected") throw mutated.reason;
			expect(normal.value, 0, /Runtime value exports match/);
			expect(
				mutated.value,
				1,
				/runtime exports absent from declarations or unexpected for this permission: isolationMarker/,
			);
			assert.notEqual(inputs.target, sibling.target);
			assert.ok((await stat(join(inputs.target, "debug", cliBinary))).isFile());
			assert.ok((await stat(join(sibling.target, "debug", cliBinary))).isFile());
			assert.deepEqual(await readdir(shared), ["sentinel"]);
			assert.equal(await readFile(marker, "utf8"), "leave this target alone");
		},
	);

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
			assert.ok((await stat(join(inputs.target, "debug", cliBinary))).isFile());
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
