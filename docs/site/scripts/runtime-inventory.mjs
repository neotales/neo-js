import { spawnSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const declarations = fileURLToPath(new URL("../../../types/neo-js.d.ts", import.meta.url));
const permissions = {
	"neo:env": ["--allow-env"],
	"neo:exec": ["--allow-run"],
	"neo:fetch": ["--allow-net"],
	"neo:fs": ["--allow-fs"],
	"neo:process": ["--allow-run", "--allow-stdio"],
	"neo:winregistry": ["--allow-registry"],
};
// Only this export disappears from an otherwise importable module.
const conditionalExports = { "neo:hash": { digestFile: "--allow-fs" } };

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
			if (!/^neo:[a-z]+(?:-[a-z]+)*$/.test(name)) {
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
			`Could not run neo-js export probe (${profile}): ${child.error?.code ?? child.signal ?? `exit ${child.status}`}. Check NEO_JS_DOCS_BIN.`,
		);
	}
	let result;
	try {
		result = JSON.parse(child.stdout);
	} catch {
		throw new Error(
			`neo-js did not return an export inventory (${profile}). Check NEO_JS_DOCS_BIN.`,
		);
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
async function checkInventory(binary) {
	const declared = declaredInventory();
	const modules = [...declared.keys()].sort();
	const flags = [...new Set(Object.values(permissions).flat())].sort();
	const problems = [];
	const directory = await mkdtemp(join(tmpdir(), "neo-js-docs-inventory-"));
	try {
		for (const profile of [flags, [], ...flags.map((flag) => [flag])]) {
			const actual = await runtimeInventory(binary, directory, modules, profile);
			for (const [index, name] of modules.entries()) {
				const context = `${name} (${profile.join(" ") || "no flags"})`;
				const required = permissions[name];
				const allowed = !required || required.some((flag) => profile.includes(flag));
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
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
	if (problems.length) throw new Error(`Runtime export inventory differs:\n${problems.join("\n")}`);
	console.log(
		`Runtime value exports match ${modules.length} declaration modules in ${flags.length + 2} permission profiles.`,
	);
}

try {
	const binary = process.env.NEO_JS_DOCS_BIN;
	if (!binary || !isAbsolute(binary)) {
		throw new Error(
			"Set NEO_JS_DOCS_BIN to the absolute path of a neo-js CLI binary built from this checkout. See docs/site/README.md.",
		);
	}
	await checkInventory(binary);
} catch (error) {
	console.error(error.message);
	process.exitCode = 1;
}
