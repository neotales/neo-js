import { spawnSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const checkout = fileURLToPath(new URL("../../../", import.meta.url));
const declarations = join(checkout, "types", "neo-js.d.ts");
const permissions = {
	"neo:env": ["--allow-env"],
	"neo:exec": ["--allow-run"],
	"neo:fetch": ["--allow-net"],
	"neo:fs": ["--allow-fs"],
	"neo:process": ["--allow-run", "--allow-stdio"],
	"neo:winregistry": ["--allow-registry"],
	// Separate imports under the CLI's unrestricted allowlist; none needs a flag.
	"neo:os/schedulers/systemd": [],
	"neo:os/schedulers/windows": [],
	"neo:scheduler": [],
};
// Only this export disappears from an otherwise importable module.
const conditionalExports = { "neo:hash": { digestFile: "--allow-fs" } };

/** Build the checkout's CLI offline and use only the executable reported by Cargo. */
function buildCli() {
	const args = [
		"build",
		"--locked",
		"--offline",
		"--manifest-path",
		"Cargo.toml",
		"--package",
		"neo-js-cli",
		"--bin",
		"neo-js",
		"--target-dir",
		"target/docs-reference",
		"--message-format=json",
	];
	const child = spawnSync("cargo", args, {
		cwd: checkout,
		encoding: "utf8",
		stdio: ["ignore", "pipe", "ignore"],
		maxBuffer: 16 * 1024 * 1024,
	});
	if (child.error || child.status !== 0) {
		throw new Error(
			`Could not build the current neo-js checkout offline (${child.error?.code ?? child.signal ?? `exit ${child.status}`}). Run cargo build --locked --offline -p neo-js-cli --bin neo-js --target-dir target/docs-reference from the checkout root for details.`,
		);
	}
	const executables = child.stdout
		.split(/\r?\n/)
		.filter((line) => line.startsWith("{"))
		.map((line) => JSON.parse(line))
		.filter(
			(artifact) =>
				artifact.reason === "compiler-artifact" &&
				artifact.target?.name === "neo-js" &&
				artifact.target.kind.includes("bin") &&
				artifact.executable,
		);
	if (executables.length !== 1)
		throw new Error("Cargo did not report exactly one neo-js CLI executable.");
	return executables[0].executable;
}

/** Check the permission model against Clap option labels, not mentions in help text. */
function checkedFlags(binary, directory) {
	const child = spawnSync(binary, ["--help"], {
		cwd: directory,
		env: {},
		encoding: "utf8",
		stdio: ["ignore", "pipe", "ignore"],
		timeout: 15_000,
	});
	if (child.error || child.status !== 0)
		throw new Error("Could not read permission flags from neo-js --help.");
	const options = child.stdout.split(/\r?\nOptions:\r?\n/)[1];
	if (!options) throw new Error("Could not find the Options section in neo-js --help.");

	const actual = new Set();
	for (const line of options.split(/\r?\n/)) {
		// Clap aligns option labels separately from their descriptions.
		const label = /^ {2}(?:-[^ ,]+, +| {0,4})(--allow-[\w-]+)(?= |,|=|$)/.exec(line);
		if (label) actual.add(label[1]);
	}
	const expected = new Set([
		...Object.values(permissions).flat(),
		...Object.values(conditionalExports).flatMap(Object.values),
	]);
	const unmodeled = [...actual].filter((flag) => !expected.has(flag)).sort();
	const unlisted = [...expected].filter((flag) => !actual.has(flag)).sort();
	if (unmodeled.length || unlisted.length) {
		throw new Error(
			`CLI permission flags differ from the inventory model. Unmodeled in check: ${unmodeled.join(", ") || "none"}. Not listed in CLI help: ${unlisted.join(", ") || "none"}. Update the permission matrix.`,
		);
	}
	return [...actual].sort();
}

/** A value exists only if each alias in its chain permits a value import. */
function isValue(symbol, checker) {
	const seen = new Set();
	while (symbol && !seen.has(symbol)) {
		if (symbol.declarations?.some(ts.isTypeOnlyImportOrExportDeclaration)) return false;
		if (!(symbol.flags & ts.SymbolFlags.Alias)) return Boolean(symbol.flags & ts.SymbolFlags.Value);
		seen.add(symbol);
		symbol = checker.getImmediateAliasedSymbol(symbol);
	}
	return false;
}

/** Distinguish value wildcard re-exports from `export type *`. */
function hasValueExport(module, exported, checker, seen = new Set()) {
	if (seen.has(module)) return false;
	const direct = module.exports?.get(exported.escapedName);
	if (direct) return isValue(direct, checker);
	seen.add(module);
	for (const declaration of module.declarations ?? []) {
		const statements = declaration.body?.statements ?? declaration.statements ?? [];
		for (const statement of statements) {
			if (
				!ts.isExportDeclaration(statement) ||
				statement.isTypeOnly ||
				statement.exportClause ||
				!statement.moduleSpecifier
			)
				continue;
			const origin = checker.getSymbolAtLocation(statement.moduleSpecifier);
			if (origin && hasValueExport(origin, exported, checker, seen)) return true;
		}
	}
	return false;
}

/** Read public value export names without counting interfaces or type aliases. */
function declaredInventory() {
	const program = ts.createProgram([declarations], {
		types: [],
		target: ts.ScriptTarget.ES2022,
		noEmit: true,
	});
	const diagnostics = ts.getPreEmitDiagnostics(program);
	if (diagnostics.length) {
		throw new Error(
			"Cannot inventory invalid TypeScript declarations:\n" +
				diagnostics
					.map(
						(diagnostic) =>
							`TS${diagnostic.code}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")}`,
					)
					.join("\n"),
		);
	}
	const checker = program.getTypeChecker();
	const modules = checker.getAmbientModules();
	if (!modules.length) throw new Error("No ambient neo: modules found in types/neo-js.d.ts.");
	return new Map(
		modules.map((module) => {
			const name = JSON.parse(module.getName());
			if (!/^neo:[a-z]+(?:[-/][a-z]+)*$/.test(name)) {
				throw new Error("Only built-in neo: modules can be imported by the inventory probe.");
			}
			const values = checker
				.getExportsOfModule(module)
				.filter((symbol) => hasValueExport(module, symbol, checker))
				.map((symbol) => symbol.getName())
				.sort();
			return [name, values];
		}),
	);
}

/** Import namespaces and return only keys. Never read values or call module functions. */
async function runtimeInventory(binary, directory, modules, flags) {
	const probe = `const names = ${JSON.stringify(modules)};
const result = [];
for (const name of names) {
	try {
		result.push(Object.getOwnPropertyNames(await import(name)).sort());
	} catch {
		result.push(null);
	}
}
console.log(JSON.stringify(result));`;
	await writeFile(join(directory, "inventory.mjs"), probe);
	const child = spawnSync(binary, [...flags, "inventory.mjs"], {
		cwd: directory,
		env: {},
		encoding: "utf8",
		stdio: ["ignore", "pipe", "ignore"],
		timeout: 15_000,
	});
	const profile = flags.join(" ") || "no flags";
	if (child.error || child.status !== 0) {
		throw new Error(
			`Could not run neo-js export probe (${profile}): ${child.error?.code ?? child.signal ?? `exit ${child.status}`}.`,
		);
	}
	let result;
	try {
		result = JSON.parse(child.stdout);
	} catch {
		throw new Error(`neo-js did not return an export inventory (${profile}).`);
	}
	if (
		!Array.isArray(result) ||
		result.length !== modules.length ||
		result.some(
			(entry) =>
				entry !== null && (!Array.isArray(entry) || entry.some((key) => typeof key !== "string")),
		)
	)
		throw new Error(`neo-js returned an invalid export inventory (${profile}).`);
	return result;
}

/** Compare namespaces and module availability in isolated CLI runs. */
async function checkInventory() {
	const declared = declaredInventory();
	const modules = [...declared.keys()].sort();
	const binary = buildCli();
	const problems = [];
	const directory = await mkdtemp(join(tmpdir(), "neo-js-docs-inventory-"));
	try {
		const flags = checkedFlags(binary, directory);
		for (const profile of [flags, [], ...flags.map((flag) => [flag])]) {
			const actual = await runtimeInventory(binary, directory, modules, profile);
			for (const [index, name] of modules.entries()) {
				const context = `${name} (${profile.join(" ") || "no flags"})`;
				const required = permissions[name];
				const allowed = !required?.length || required.some((flag) => profile.includes(flag));
				const keys = actual[index];
				if (!allowed) {
					if (keys !== null) problems.push(`${context}: imported without a required permission.`);
					continue;
				}
				if (keys === null) {
					problems.push(`${context}: module failed to import.`);
					continue;
				}
				const gates = conditionalExports[name] ?? {};
				const expected = declared
					.get(name)
					.filter((key) => !Object.hasOwn(gates, key) || profile.includes(gates[key]));
				const missing = expected.filter((key) => !keys.includes(key));
				const extra = keys.filter((key) => !expected.includes(key));
				if (missing.length)
					problems.push(`${context}: declared but missing at runtime: ${missing.join(", ")}.`);
				if (extra.length)
					problems.push(
						`${context}: runtime exports absent from declarations or unexpected for this permission: ${extra.join(", ")}.`,
					);
			}
		}
		if (problems.length)
			throw new Error(`Runtime export inventory differs:\n${problems.join("\n")}`);
		console.log(
			`Runtime value exports match ${modules.length} declaration modules in ${flags.length + 2} permission profiles.`,
		);
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
}

try {
	await checkInventory();
} catch (error) {
	console.error(error.message);
	process.exitCode = 1;
}
